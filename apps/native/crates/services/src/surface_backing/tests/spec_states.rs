//! Follow-up assertions for Opus specs merged at ce36cb6fc.
use super::*;
use crate::surface_backing::{
    authorization::*,
    copy,
    generated::*,
    maintenance::*,
    native::{colour::*, tasks::*},
    projects::*,
};
use pm_promedia::{
    scopes::ScopeConfig,
    tasks::{BackgroundTasks, ProgressUnit, TaskFailureCode, TaskKind, TaskSpec},
};

#[test]
fn notes_readonly_and_focused_external_change_follow_spec() {
    let s = session(1);
    let mut n = Notes::default();
    n.attach(s.clone(), "a".into());
    n.set_read_only(&s, true).unwrap();
    assert!(n.input(&s, "refused".into(), 0).is_err());
    assert_eq!(
        copy::notes_status(n.draft(&s).unwrap(), true),
        "This project is read-only."
    );
    n.set_read_only(&s, false).unwrap();
    n.focus(&s, true).unwrap();
    n.input(&s, "local".into(), 0).unwrap();
    n.observe(&s, "remote".into()).unwrap();
    let w = n.flush(&s, 0, true).unwrap().unwrap();
    assert_eq!(w.expected, "remote");
    assert_eq!(w.text, "local");
}
#[test]
fn take_delete_undo_keeps_archive_and_refuses_intervening_revision() {
    let before = records(vec![take_record(
        "t".into(),
        "Authored".into(),
        1,
        "{}".into(),
    )]);
    let mut h = MemoryRecords {
        current: Some(before.clone()),
        ..Default::default()
    };
    let revision = archive_take(&mut h, &before, "t").unwrap();
    let current = h.current.clone().unwrap();
    undo_take_delete(&mut h, &current, &before, revision).unwrap();
    assert_eq!(h.history.len(), 2);
    assert_eq!(h.current.as_ref().unwrap().entries, before.entries);
    let later = h.current.clone().unwrap();
    assert!(undo_take_delete(&mut h, &later, &before, revision).is_err());
}
#[test]
fn numeric_generated_edits_clamp_but_imported_control_is_unchanged() {
    let c = json!({"type":"number","label":"Amount","min":0,"max":1,"def":4,"unknown":5});
    assert_eq!(edit_value(&c, json!(4)).unwrap(), 1.);
    assert_eq!(c["def"], 4);
    assert!(matches!(binding(&c).unwrap(), Binding::Parameter(_)));
    assert_eq!(
        binding(&json!({"type":"note","label":"Authored text"})).unwrap(),
        Binding::Note
    );
}
#[test]
fn generated_apply_accepts_only_its_published_revision_and_preview_cannot_mutate() {
    let definition = json!({"id":"p","controls":[]});
    let mut p = GeneratedPanel::new(
        definition.clone(),
        PanelScope {
            workspace_id: "workspace".into(),
            revision: 1,
        },
    )
    .unwrap();
    let r = p
        .begin(
            session(1),
            7,
            &json!({"action":{"type":"script","mode":"apply"}}),
        )
        .unwrap();
    p.settle(
        &r,
        &session(1),
        8,
        Ok(ToolReply {
            result: json!({"ok":true}),
            published_revision: 8,
            applied: true,
        }),
    )
    .unwrap();
    assert!(p.preview.is_some());
    let mut p = GeneratedPanel::new(
        definition,
        PanelScope {
            workspace_id: "workspace".into(),
            revision: 1,
        },
    )
    .unwrap();
    let r = p
        .begin(
            session(1),
            7,
            &json!({"action":{"type":"script","mode":"preview"}}),
        )
        .unwrap();
    assert!(
        p.settle(
            &r,
            &session(1),
            8,
            Ok(ToolReply {
                result: json!({"ok":true}),
                published_revision: 8,
                applied: true
            })
        )
        .is_err()
    );
    assert!(p.preview.is_none());
}
#[test]
fn scopes_keep_previous_result_waiting_and_limit_playback_sampling_to_10_hz() {
    let f = FrameIdentity {
        session: session(1),
        revision: 2,
        frame: 1,
        colour_pipeline: "p".into(),
    };
    let mut a = ColourAnalysis::default();
    let r = a
        .try_begin(
            f.clone(),
            AnalysisStage::DisplayEncoded,
            ScopeConfig::default(),
            0,
            true,
            true,
        )
        .unwrap();
    let data = compute(&r, AnalysisStage::DisplayEncoded, &[[0.; 3]], 1, 1).unwrap();
    a.settle(&r, &f, Ok(data)).unwrap();
    assert!(
        a.try_begin(
            f.clone(),
            AnalysisStage::DisplayEncoded,
            ScopeConfig::default(),
            99,
            true,
            true
        )
        .is_none()
    );
    assert!(
        a.try_begin(
            f.clone(),
            AnalysisStage::DisplayEncoded,
            ScopeConfig::default(),
            100,
            true,
            false
        )
        .is_none()
    );
    let r = a
        .try_begin(
            f.clone(),
            AnalysisStage::DisplayEncoded,
            ScopeConfig::default(),
            100,
            true,
            true,
        )
        .unwrap();
    assert!(a.previous.is_some());
    a.settle(&r, &f, Err(failure())).unwrap_err();
    assert!(a.previous.is_none());
}
#[test]
fn one_task_view_cannot_leak_previous_window_rows() {
    let mut q = BackgroundTasks::default();
    q.enqueue(
        "one".into(),
        "p".into(),
        TaskSpec {
            title: "Authored".into(),
            kind: TaskKind::Import,
            unit: ProgressUnit::Items,
            priority: 0,
            total: 0,
        },
    )
    .unwrap();
    let mut rows = TaskRows::default();
    rows.refresh(&q, "one");
    assert_eq!(rows.rows.len(), 1);
    rows.refresh(&q, "two");
    assert!(rows.rows.is_empty());
}
struct Os {
    status: Authorization,
    prompts: usize,
    sends: usize,
}
impl AuthorizationHost for Os {
    fn status(&mut self, _: Capability, _: &str) -> Result<Authorization> {
        Ok(self.status)
    }
    fn request(&mut self, _: &AuthorizationRequest) -> Result<Authorization> {
        self.prompts += 1;
        self.status = Authorization::Authorized;
        Ok(self.status)
    }
}
impl NotificationHost for Os {
    fn deliver(&mut self, _: &CompletionNotice) -> Result<()> {
        self.sends += 1;
        Ok(())
    }
    fn activate_thread(&mut self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
}
#[test]
fn notification_permission_asks_on_first_sent_run_and_provisional_is_allowed() {
    let mut n = Notifications::default();
    let mut os = Os {
        status: Authorization::NotDetermined,
        prompts: 0,
        sends: 0,
    };
    n.run_sent(&mut os).unwrap();
    n.run_sent(&mut os).unwrap();
    assert_eq!(os.prompts, 1);
    os.status = Authorization::Provisional;
    let notice = CompletionNotice::finished("e".into(), "r".into(), "t".into(), "w".into());
    assert_eq!(
        n.complete(
            &mut os,
            &notice,
            NotificationContext {
                enabled: true,
                frontmost: true,
                agent_visible: false,
                stopped_by_user: false
            }
        )
        .unwrap(),
        Delivery::Sent
    );
    assert_eq!(notice.identifier(), "t");
    assert_eq!(notice.sound(), "Little Victory (Deep)");
}
#[test]
fn question_and_completion_in_same_run_are_distinct_thread_replacements() {
    let mut n = Notifications::default();
    let mut os = Os {
        status: Authorization::Authorized,
        prompts: 0,
        sends: 0,
    };
    let question = CompletionNotice::question(
        "q".into(),
        "r".into(),
        "t".into(),
        "w".into(),
        "Which\noption?",
    );
    let done = CompletionNotice::finished("done".into(), "r".into(), "t".into(), "w".into());
    let context = NotificationContext {
        enabled: true,
        frontmost: false,
        agent_visible: false,
        stopped_by_user: false,
    };
    assert_eq!(question.body, "Which option?");
    n.complete(&mut os, &question, context).unwrap();
    n.complete(&mut os, &done, context).unwrap();
    assert_eq!(os.sends, 2);
    assert_eq!(
        n.complete(
            &mut os,
            &done,
            NotificationContext {
                stopped_by_user: true,
                ..context
            }
        )
        .unwrap(),
        Delivery::Stopped
    );
}
#[test]
fn forbidden_extension_requests_do_not_need_system_authorization() {
    for c in [
        ForbiddenExtensionCapability::Camera,
        ForbiddenExtensionCapability::Microphone,
        ForbiddenExtensionCapability::Screen,
    ] {
        assert_eq!(extension_request(c), Authorization::Denied);
    }
    assert!(!std::hint::black_box(COLOUR_SAMPLER_REQUIRES_AUTHORIZATION));
}
#[test]
fn copy_matches_approved_failure_and_archive_states() {
    assert_eq!(
        copy::take_saved(Some("Take 25"), 1),
        "Saved Take 25 · Oldest take archived"
    );
    assert_eq!(
        copy::task_failure(TaskFailureCode::Unknown),
        Some("Something went wrong. Details are in Window ▸ Error Log.")
    );
    assert_eq!(
        copy::notification_detail(Authorization::Denied),
        "Notifications are off for Powermove in System Settings."
    );
    assert_eq!(copy::PERFORMANCE_LABELS[2], ("draw_calls", "Draw Calls"));
    assert_eq!(
        copy::update_status(&UpdateState::Checking, "1.0.0", false),
        None
    );
    assert_eq!(
        copy::workspace_json_error("expected value"),
        "This isn’t valid JSON. expected value"
    );
}

#[test]
fn queued_tasks_offer_cancel_only_and_terminal_popover_expiry_does_not_retire_data() {
    let mut q = BackgroundTasks::default();
    let id = q
        .enqueue(
            "w".into(),
            "p".into(),
            TaskSpec {
                title: "T".into(),
                kind: TaskKind::Import,
                unit: ProgressUnit::Items,
                priority: 0,
                total: 10,
            },
        )
        .unwrap();
    assert!(act(&mut q, "w", id, TaskAction::Pause).is_err());
    let t = q.start_next().unwrap();
    let mut timing = TaskTiming::default();
    timing.observe(q.get(id).unwrap(), 0);
    q.progress(t, 5, 10).unwrap();
    assert_eq!(timing.seconds_left(q.get(id).unwrap(), 4999), None);
    assert_eq!(timing.seconds_left(q.get(id).unwrap(), 5000), Some(5));
    q.finish(t, Ok(())).unwrap();
    timing.observe(q.get(id).unwrap(), 5000);
    assert!(timing.visible(q.get(id).unwrap(), 9999, TaskContainer::Popover));
    assert!(!timing.visible(q.get(id).unwrap(), 10000, TaskContainer::Popover));
    assert!(timing.visible(q.get(id).unwrap(), 10000, TaskContainer::Window));
    assert!(q.get(id).is_some());
}

#[test]
fn update_ready_requires_verification_stage_and_automatic_skips_remain_manual_checkable() {
    use crate::updater::{FeedMetadata, FeedRoute, TransitionGate, UpdateFeed};
    struct Feed;
    impl UpdateFeed for Feed {
        fn metadata(&mut self, _: FeedRoute) -> Result<Option<FeedMetadata>> {
            Ok(Some(FeedMetadata {
                version: "2.0.0".into(),
                name: "Release".into(),
                size_bytes: 1,
                minimum_macos: 14,
                notes: None,
            }))
        }
    }
    struct Unverified;
    impl UpdateHost for Unverified {
        fn stage_verified(
            &mut self,
            _: &FeedMetadata,
            _: &mut dyn FnMut(UpdateStage) -> Result<()>,
        ) -> Result<String> {
            Ok("download-only-receipt".into())
        }
        fn request_normal_quit(&mut self) -> Result<bool> {
            panic!("must not quit")
        }
    }
    let gate = TransitionGate {
        packaged_macos: true,
        macos_major: 14,
        migration_complete: true,
        transition_release_verified: true,
    };
    let mut u = Updates::default();
    let r = u.begin_check(true).unwrap();
    u.check(&r, gate, "1.0.0", &mut Feed).unwrap();
    assert_eq!(u.skip_available().unwrap(), "2.0.0");
    let r = u.begin_check(false).unwrap();
    u.check(&r, gate, "1.0.0", &mut Feed).unwrap();
    assert_eq!(u.state, UpdateState::Current);
    let r = u.begin_check(true).unwrap();
    u.check(&r, gate, "1.0.0", &mut Feed).unwrap();
    assert!(u.stage(&mut Unverified).is_err());
    assert_eq!(
        u.state,
        UpdateState::VerificationFailed(ServiceErrorCode::InvalidData)
    );
    assert!(!u.cancellation_allowed());
}

#[test]
fn main_reference_point_axes_keep_independent_authored_steps_and_anchored_rounding() {
    use crate::surface_backing::generated_points::AxisDomain;
    let x = AxisDomain::new(-1., 1., Some(0.125)).unwrap();
    let y = AxisDomain::new(10., 30., Some(2.)).unwrap();
    assert_eq!(x.quantize(0.19).unwrap(), 0.25);
    assert_eq!(y.quantize(13.).unwrap(), 14.);
    assert_eq!(x.quantize(-2.).unwrap(), -1.);
    assert_eq!(AxisDomain::new(0., 20., None).unwrap().step, 0.2);
    assert_eq!(AxisDomain::new(0., 20., Some(f64::NAN)).unwrap().step, 0.2);
    assert!(x.quantize(f64::NAN).is_err());
    let binary_half = AxisDomain::new(0., 1., Some(1. / 128.)).unwrap();
    assert_eq!(binary_half.quantize(1. / 128.).unwrap(), 0.007813);
}
#[test]
fn main_reference_generated_groups_use_actual_controls_and_do_not_pair_local_with_source() {
    use crate::surface_backing::generated_points::groups;
    let controls = vec![
        json!({"type":"slider","label":"Position X","stateKey":"x","min":0,"max":10}),
        json!({"type":"slider","label":"Position Y","stateKey":"y","min":0,"max":10}),
        json!({"type":"color","label":"Start Color"}),
        json!({"type":"color","label":"End Color"}),
        json!({"type":"text","label":"fifth"}),
        json!({"type":"toggle","label":"sixth"}),
        json!({"type":"button","label":"Run"}),
    ];
    let g = groups(&controls).unwrap();
    assert_eq!(g.point_pairs, vec![(0, 1)]);
    assert_eq!(g.gradient_pair, Some((2, 3)));
    assert_eq!(g.secondary, vec![5]);
    let mut changed = controls;
    changed[1].as_object_mut().unwrap().remove("stateKey");
    assert!(groups(&changed).unwrap().point_pairs.is_empty());
}
#[test]
fn main_reference_unchanged_meter_size_preserves_backing_and_scale_change_invalidates_once() {
    use crate::surface_backing::mixer::MeterBacking;
    let mut backing = MeterBacking::new(12., 30., 2.).unwrap();
    assert!(!backing.replace_if_changed(MeterBacking::new(12., 30., 2.).unwrap()));
    assert!(backing.replace_if_changed(MeterBacking::new(12., 30., 1.).unwrap()));
    assert!(!backing.replace_if_changed(MeterBacking::new(12., 30., 1.).unwrap()));
    assert_eq!(backing.pixels, [12, 30]);
    assert!(MeterBacking::new(f64::NAN, 0., 1.).is_err());
}
#[test]
fn main_reference_3d_context_is_empty_only_and_studio_upgrade_cannot_fake_success() {
    use crate::surface_backing::scene::*;
    assert!(add_menu_available(None));
    assert!(add_menu_available(Some("empty")));
    assert!(!add_menu_available(Some("layer")));
    struct Absent;
    impl SceneUpgradeHost for Absent {}
    assert!(Absent.upgrade_and_inspect(&session(1), 7, "layer").is_err());
    let material = studio_fields(&json!({"object":{}})).unwrap();
    assert_eq!(
        material.iter().find(|f| f.path == "m.ior").unwrap().max,
        Some(2.6)
    );
    let area = studio_fields(&json!({"light":{"type":"area"}})).unwrap();
    assert!(area.iter().any(|f| f.path == "light.width"));
    assert!(!area.iter().any(|f| f.path == "light.decay"));
}

#[test]
fn generated_local_flush_targets_captured_workspace_and_acknowledges_its_revision() {
    struct Host {
        saved: Option<String>,
    }
    impl GeneratedHost for Host {
        fn run(&mut self, _: &ToolRequest) -> Result<ToolReply> {
            Err(unavailable("not used"))
        }
        fn save_local(&mut self, scope: &PanelScope, _: &str, _: &Value) -> Result<u64> {
            self.saved = Some(scope.workspace_id.clone());
            Ok(scope.revision + 1)
        }
    }
    let mut panel = GeneratedPanel::new(
        json!({"id":"p","controls":[],"state":{}}),
        PanelScope {
            workspace_id: "old-workspace".into(),
            revision: 4,
        },
    )
    .unwrap();
    let mut host = Host { saved: None };
    panel.set_local("x", json!(5)).unwrap();
    panel.flush_local(&mut host).unwrap();
    assert_eq!(host.saved.as_deref(), Some("old-workspace"));
    assert_eq!(panel.scope.revision, 5);
    let request = panel
        .begin(
            session(1),
            7,
            &json!({"label":"Authored Undo Name","action":{"type":"script","mode":"apply"}}),
        )
        .unwrap();
    assert_eq!(request.label.as_deref(), Some("Authored Undo Name"));
    assert_eq!(request.panel_id, "p");
    assert_eq!(request.workspace.workspace_id, "old-workspace");
}
