use super::*;
use crate::surface_backing::{authorization::*, maintenance::*, onboarding::*};
use crate::updater::{FeedMetadata, FeedRoute, TransitionGate, UpdateFeed};
use std::collections::BTreeMap;

struct WelcomeHost {
    calls: Vec<&'static str>,
    fail_import: bool,
    fail_marker: bool,
    fail_preference: bool,
}
impl OnboardingHost for WelcomeHost {
    fn create_editor(&mut self) -> Result<()> {
        self.calls.push("editor");
        Ok(())
    }
    fn timeline(&mut self, _: Timeline) -> Result<()> {
        self.calls.push("timeline");
        if self.fail_preference {
            Err(failure())
        } else {
            Ok(())
        }
    }
    fn agent(&mut self, _: &Agent) -> Result<()> {
        self.calls.push("agent");
        Ok(())
    }
    fn import_workspace(&mut self) -> Result<()> {
        self.calls.push("import");
        if self.fail_import {
            Err(failure())
        } else {
            Ok(())
        }
    }
    fn completed(&mut self, record: &Value) -> Result<()> {
        self.calls.push("marker");
        assert_eq!(record["version"], 1);
        assert_eq!(record["future"], 9);
        if self.fail_marker {
            Err(failure())
        } else {
            Ok(())
        }
    }
    fn download_transcription(&mut self, _: &str) -> Result<()> {
        self.calls.push("download");
        Ok(())
    }
}
fn choice() -> Choice {
    Choice::parse(json!({"workspaceImport":"after-effects","timelineMode":"tracks","agent":{"provider":"chatgpt","model":"model/test"},"future":9})).unwrap()
}
fn welcome() -> Onboarding {
    let mut o = Onboarding::new(true);
    o.welcome();
    for _ in 0..4 {
        o.advance().unwrap();
    }
    o
}
fn welcome_host() -> WelcomeHost {
    WelcomeHost {
        calls: vec![],
        fail_import: false,
        fail_marker: false,
        fail_preference: false,
    }
}
#[test]
fn onboarding_choices_validate_tagged_domains_and_keep_future_keys() {
    assert_eq!(choice().raw["future"], 9);
    assert_eq!(choice().timeline, Timeline::Tracks);
    for v in [
        json!({"workspaceImport":"unknown"}),
        json!({"workspaceImport":null,"timelineMode":4}),
        json!({"workspaceImport":null,"agent":{"provider":"chatgpt","model":"bad model"}}),
    ] {
        assert!(Choice::parse(v).is_err());
    }
}
#[test]
fn onboarding_runs_editor_preferences_agent_import_marker_in_order() {
    let mut o = welcome();
    let r = o.begin(choice()).unwrap();
    let mut host = welcome_host();
    let done = execute(&mut host, &r).unwrap();
    o.settle(&r, Ok(done)).unwrap();
    assert_eq!(
        host.calls,
        vec!["editor", "timeline", "agent", "import", "marker"]
    );
    assert_eq!(o.step, Step::Complete);
    assert!(!o.first_run_pending);
}
#[test]
fn onboarding_preference_and_marker_failures_leave_editor_usable_with_explicit_warnings() {
    let mut o = welcome();
    let r = o.begin(choice()).unwrap();
    let mut host = welcome_host();
    host.fail_marker = true;
    host.fail_preference = true;
    let done = execute(&mut host, &r).unwrap();
    assert!(!done.marker_written);
    assert_eq!(done.warnings, vec![ServiceErrorCode::DiskFull; 2]);
    o.settle(&r, Ok(done)).unwrap();
    assert!(!o.first_run_pending);
}
#[test]
fn onboarding_import_failure_does_not_publish_completion_and_can_retry() {
    let mut o = welcome();
    let r = o.begin(choice()).unwrap();
    let mut host = welcome_host();
    host.fail_import = true;
    let result = execute(&mut host, &r);
    assert!(o.settle(&r, result).is_err());
    assert_eq!(o.step, Step::Workspace);
    assert!(o.first_run_pending);
    assert!(!host.calls.contains(&"marker"));
    let next = o.begin(choice()).unwrap();
    assert_ne!(next.ticket, r.ticket);
    assert!(
        o.settle(
            &r,
            Ok(Completion {
                warnings: vec![],
                marker_written: true
            })
        )
        .is_err()
    );
}
#[test]
fn onboarding_transcription_download_outlives_navigation_and_cannot_be_replayed_mid_import() {
    let mut host = welcome_host();
    host.download_transcription("speech").unwrap();
    let mut o = welcome();
    o.back().unwrap();
    assert_eq!(o.step, Step::Transcription);
    o.advance().unwrap();
    o.begin(choice()).unwrap();
    assert!(o.replay().is_err());
    assert_eq!(host.calls, vec!["download"]);
}
#[test]
fn workspace_handoff_never_submits_changed_draft_wrong_thread_or_running_agent() {
    let h = ImportHandoff {
        thread_id: "t".into(),
        original_draft: "authored request".into(),
    };
    assert!(h.may_resume("t", "app", "authored request", false));
    assert!(!h.may_resume("other", "app", "authored request", false));
    assert!(!h.may_resume("t", "app", "changed", false));
    assert!(!h.may_resume("t", "project", "authored request", false));
    assert!(!h.may_resume("t", "app", "authored request", true));
}
fn metadata() -> FeedMetadata {
    FeedMetadata {
        version: "2.0.0".into(),
        name: "Authored release".into(),
        size_bytes: 123,
        minimum_macos: 14,
        notes: Some("untrusted notes".into()),
    }
}
fn gate() -> TransitionGate {
    TransitionGate {
        packaged_macos: true,
        macos_major: 14,
        migration_complete: true,
        transition_release_verified: true,
    }
}
struct Feed {
    calls: u32,
    fail: bool,
}
impl UpdateFeed for Feed {
    fn metadata(&mut self, _: FeedRoute) -> Result<Option<FeedMetadata>> {
        self.calls += 1;
        if self.fail {
            Err(failure())
        } else {
            Ok(Some(metadata()))
        }
    }
}
struct Installer {
    fail: bool,
    quits: u32,
}
impl UpdateHost for Installer {
    fn stage_verified(
        &mut self,
        _: &FeedMetadata,
        report: &mut dyn FnMut(UpdateStage) -> Result<()>,
    ) -> Result<String> {
        if self.fail {
            Err(failure())
        } else {
            report(UpdateStage::Download {
                done: 123,
                total: 123,
            })?;
            report(UpdateStage::Verifying)?;
            Ok("synthetic-verified-staging-receipt".into())
        }
    }
    fn request_normal_quit(&mut self) -> Result<bool> {
        self.quits += 1;
        Ok(false)
    }
}
fn available() -> Updates {
    let mut u = Updates::default();
    let r = u.begin_check(false).unwrap();
    u.check(
        &r,
        gate(),
        "1.0.0",
        &mut Feed {
            calls: 0,
            fail: false,
        },
    )
    .unwrap();
    u
}
#[test]
fn gated_update_never_contacts_feed() {
    let mut u = Updates::default();
    let r = u.begin_check(true).unwrap();
    let mut f = Feed {
        calls: 0,
        fail: false,
    };
    u.check(&r, TransitionGate::default(), "1.0.0", &mut f)
        .unwrap();
    assert_eq!(f.calls, 0);
    assert_eq!(u.state, UpdateState::Gated);
}
#[test]
fn updater_requires_verified_staging_before_normal_quit_barrier() {
    let mut u = available();
    let mut installer = Installer {
        fail: false,
        quits: 0,
    };
    assert!(u.install(&mut installer).is_err());
    u.stage(&mut installer).unwrap();
    assert!(matches!(u.state, UpdateState::Ready { .. }));
    assert!(!u.install(&mut installer).unwrap());
    assert_eq!(installer.quits, 1);
    assert!(matches!(u.state, UpdateState::Ready { .. }));
}
#[test]
fn update_staging_and_feed_failures_cannot_be_announced_ready() {
    let mut u = available();
    assert!(
        u.stage(&mut Installer {
            fail: true,
            quits: 0
        })
        .is_err()
    );
    assert_eq!(
        u.state,
        UpdateState::DownloadFailed(ServiceErrorCode::DiskFull)
    );
    let r = u.begin_check(true).unwrap();
    assert!(
        u.check(
            &r,
            gate(),
            "1.0.0",
            &mut Feed {
                calls: 0,
                fail: true
            }
        )
        .is_err()
    );
    assert!(!u.notice(true));
}
#[test]
fn update_notice_dismissal_is_per_version_and_home_still_reports_ready() {
    let mut u = available();
    u.stage(&mut Installer {
        fail: false,
        quits: 0,
    })
    .unwrap();
    assert!(u.notice(false));
    assert_eq!(u.dismiss().unwrap(), "2.0.0");
    assert!(!u.notice(false));
    assert!(u.notice(true));
}
#[test]
fn stale_update_callbacks_cannot_replace_current_check() {
    let mut u = Updates::default();
    let r = u.begin_check(false).unwrap();
    assert!(u.begin_check(true).is_err());
    u.check(
        &r,
        gate(),
        "1.0.0",
        &mut Feed {
            calls: 0,
            fail: false,
        },
    )
    .unwrap();
    assert!(
        u.check(
            &r,
            gate(),
            "1.0.0",
            &mut Feed {
                calls: 0,
                fail: false
            }
        )
        .is_err()
    );
}
fn release(v: &str) -> Release {
    Release {
        version: v.into(),
        draft: false,
        prerelease: v.contains("beta"),
        metadata: json!({"notes":"<script>data, never execute</script>","future":1}),
    }
}
struct ReleaseStore {
    seen: Option<String>,
    fail_fetch: bool,
    fail_write: bool,
    writes: Vec<String>,
}
impl ReleaseHost for ReleaseStore {
    fn seen_version(&mut self) -> Result<Option<String>> {
        Ok(self.seen.clone())
    }
    fn releases(&mut self) -> Result<Vec<Release>> {
        if self.fail_fetch {
            Err(failure())
        } else {
            Ok(vec![release("1.1.0"), release("1.2.0")])
        }
    }
    fn mark_seen(&mut self, v: &str) -> Result<()> {
        if self.fail_write {
            return Err(failure());
        }
        self.writes.push(v.into());
        self.seen = Some(v.into());
        Ok(())
    }
}
fn release_store() -> ReleaseStore {
    ReleaseStore {
        seen: Some("1.0.0".into()),
        fail_fetch: false,
        fail_write: false,
        writes: vec![],
    }
}
#[test]
fn release_filter_uses_semantic_bounds_beta_policy_and_preserves_notes_as_data() {
    let mut draft = release("1.1.5");
    draft.draft = true;
    let input = vec![
        release("1.2.0"),
        release("1.1.0"),
        release("1.2.0-beta.1"),
        release("1.3.0"),
        release("malformed"),
        draft,
    ];
    let stable = releases_between(&input, "1.0.0", "1.2.0").unwrap();
    assert_eq!(
        stable
            .iter()
            .map(|r| r.version.as_str())
            .collect::<Vec<_>>(),
        vec!["1.2.0", "1.1.0"]
    );
    assert_eq!(stable[0].metadata["future"], 1);
    let beta = releases_between(&input, "1.1.0", "1.2.0-beta.1").unwrap();
    assert_eq!(beta.len(), 1);
    assert_eq!(beta[0].version, "1.2.0-beta.1");
}
#[test]
fn fresh_install_records_version_without_showing_notes() {
    let mut store = release_store();
    store.seen = None;
    let mut w = WhatsNew::default();
    assert!(w.pending(&mut store, "window", "1.2.0").unwrap().is_none());
    assert_eq!(store.writes, vec!["1.2.0"]);
}
#[test]
fn release_claim_is_window_scoped_and_seen_only_after_acknowledged_show() {
    let mut store = release_store();
    let mut w = WhatsNew::default();
    let claim = w.pending(&mut store, "one", "1.2.0").unwrap().unwrap();
    assert!(store.writes.is_empty());
    assert!(w.pending(&mut store, "two", "1.2.0").unwrap().is_none());
    let mut forged = claim.clone();
    forged.window_id = "two".into();
    assert!(w.shown(&mut store, &forged).is_err());
    store.fail_write = true;
    assert!(w.shown(&mut store, &claim).is_err());
    store.fail_write = false;
    w.shown(&mut store, &claim).unwrap();
    assert_eq!(store.writes, vec!["1.2.0"]);
}
#[test]
fn offline_release_fetch_and_lost_window_remain_retryable() {
    let mut store = release_store();
    let mut w = WhatsNew::default();
    store.fail_fetch = true;
    assert!(w.pending(&mut store, "one", "1.2.0").is_err());
    store.fail_fetch = false;
    let old = w.pending(&mut store, "one", "1.2.0").unwrap().unwrap();
    w.window_lost("one");
    let new = w.pending(&mut store, "two", "1.2.0").unwrap().unwrap();
    assert_ne!(old.ticket, new.ticket);
    assert!(w.shown(&mut store, &old).is_err());
    assert!(store.writes.is_empty());
}
#[test]
fn native_diagnostics_throttle_visibility_and_missing_metrics_are_explicit() {
    let mut p = Performance {
        visible: true,
        ..Default::default()
    };
    let metrics = BTreeMap::from([
        ("gpu_completion_ms".into(), Metric::Milliseconds(3.)),
        (
            "draws".into(),
            Metric::Unavailable(ServiceErrorCode::Unavailable),
        ),
    ]);
    assert!(
        p.observe(PerformanceSample {
            at_ms: 10,
            metrics: metrics.clone()
        })
        .unwrap()
    );
    assert!(
        !p.observe(PerformanceSample {
            at_ms: 509,
            metrics: metrics.clone()
        })
        .unwrap()
    );
    assert!(
        p.observe(PerformanceSample {
            at_ms: 510,
            metrics: metrics.clone()
        })
        .unwrap()
    );
    p.visible = false;
    assert!(
        !p.observe(PerformanceSample {
            at_ms: 1010,
            metrics
        })
        .unwrap()
    );
    assert!(
        p.observe(PerformanceSample {
            at_ms: 2000,
            metrics: BTreeMap::from([("invalid".into(), Metric::PerSecond(f64::NAN))])
        })
        .is_err()
    );
}
struct Os {
    status: Authorization,
    requests: u32,
    sent: u32,
    fail_send: bool,
    activated: Vec<(String, String)>,
}
impl AuthorizationHost for Os {
    fn status(&mut self, _: Capability, _: &str) -> Result<Authorization> {
        Ok(self.status)
    }
    fn request(&mut self, _: &AuthorizationRequest) -> Result<Authorization> {
        self.requests += 1;
        Ok(self.status)
    }
}
impl NotificationHost for Os {
    fn deliver(&mut self, _: &CompletionNotice) -> Result<()> {
        if self.fail_send {
            return Err(failure());
        }
        self.sent += 1;
        Ok(())
    }
    fn activate_thread(&mut self, w: &str, t: &str) -> Result<()> {
        self.activated.push((w.into(), t.into()));
        Ok(())
    }
}
fn os() -> Os {
    Os {
        status: Authorization::Authorized,
        requests: 0,
        sent: 0,
        fail_send: false,
        activated: vec![],
    }
}
fn notice() -> CompletionNotice {
    CompletionNotice {
        event_id: "e".into(),
        question: false,
        run_id: "r".into(),
        thread_id: "t".into(),
        window_id: "w".into(),
        title: "supplied title".into(),
        body: "supplied body".into(),
    }
}
#[test]
fn authorization_denial_cancellation_restriction_and_unavailable_remain_distinct() {
    for expected in [
        Authorization::Denied,
        Authorization::Cancelled,
        Authorization::Restricted,
        Authorization::Unavailable,
        Authorization::NotDetermined,
    ] {
        let mut f = AuthorizationFlow::default();
        let r = f
            .begin(Some(session(1)), Capability::FileAccess, "sampler".into())
            .unwrap();
        assert_eq!(
            f.settle(&r, Some(&session(1)), Ok(expected)).unwrap(),
            expected
        );
        assert_eq!(f.outcome, Some(expected));
    }
}
#[test]
fn authorization_callbacks_cannot_cross_project_reopen_or_detach() {
    let mut f = AuthorizationFlow::default();
    let r = f
        .begin(
            Some(session(1)),
            Capability::SelectedFile,
            "chosen-bookmark".into(),
        )
        .unwrap();
    assert!(
        f.settle(&r, Some(&session(2)), Ok(Authorization::Authorized))
            .is_err()
    );
    assert!(f.outcome.is_none());
    let r = f.begin(None, Capability::Notifications, "".into()).unwrap();
    f.detach();
    assert!(f.settle(&r, None, Ok(Authorization::Authorized)).is_err());
}
#[test]
fn notification_completion_never_prompts_and_failed_delivery_can_retry() {
    let mut h = os();
    let mut n = Notifications::default();
    h.status = Authorization::Denied;
    assert_eq!(
        n.complete(&mut h, &notice(), notification_context(true, false))
            .unwrap(),
        Delivery::Authorization(Authorization::Denied)
    );
    assert_eq!(h.requests, 0);
    h.status = Authorization::Authorized;
    h.fail_send = true;
    assert!(
        n.complete(&mut h, &notice(), notification_context(true, false))
            .is_err()
    );
    h.fail_send = false;
    assert_eq!(
        n.complete(&mut h, &notice(), notification_context(true, false))
            .unwrap(),
        Delivery::Sent
    );
    assert_eq!(
        n.complete(&mut h, &notice(), notification_context(true, false))
            .unwrap(),
        Delivery::Duplicate
    );
    assert_eq!(h.sent, 1);
    n.activate(&mut h, &notice()).unwrap();
    assert_eq!(h.activated, vec![("w".into(), "t".into())]);
}
#[test]
fn disabled_foreground_notifications_never_call_os_delivery() {
    let mut h = os();
    let mut n = Notifications::default();
    assert_eq!(
        n.complete(&mut h, &notice(), notification_context(false, false))
            .unwrap(),
        Delivery::Disabled
    );
    assert_eq!(
        n.complete(&mut h, &notice(), notification_context(true, true))
            .unwrap(),
        Delivery::Foreground
    );
    assert_eq!(h.sent, 0);
    assert!(n.activate(&mut h, &notice()).is_err());
}

fn notification_context(enabled: bool, visible: bool) -> NotificationContext {
    NotificationContext {
        enabled,
        frontmost: visible,
        agent_visible: visible,
        stopped_by_user: false,
    }
}
