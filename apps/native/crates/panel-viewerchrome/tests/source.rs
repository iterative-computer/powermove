use pm_panel_viewerchrome::*;
fn video(id: &str) -> Option<SourceAsset> {
    Some(SourceAsset {
        id: id.into(),
        kind: SourceKind::Video,
        available: true,
        duration: Some(12.5),
    })
}
#[test]
fn showing_source_pauses_composition_reuses_same_id_and_switch_releases_old() {
    let plan = SourceState::default()
        .plan(SourceCommand::Show("a".into()), video)
        .unwrap();
    assert_eq!(
        plan.effects,
        vec![
            SourceEffect::Load("a".into()),
            SourceEffect::PauseComposition
        ]
    );
    assert!(!plan.next.playing);
    let same = plan
        .next
        .plan(SourceCommand::Show("a".into()), |_| {
            panic!("same ID must reuse")
        })
        .unwrap();
    assert!(same.effects.is_empty());
    let next = same
        .next
        .plan(SourceCommand::Show("b".into()), video)
        .unwrap();
    assert_eq!(
        next.effects,
        vec![
            SourceEffect::Release("a".into()),
            SourceEffect::Load("b".into()),
            SourceEffect::PauseComposition
        ]
    );
    let closed = next.next.plan(SourceCommand::Close, video).unwrap();
    assert!(closed.next.asset.is_none());
    assert!(!closed.next.playing);
    assert_eq!(closed.effects, vec![SourceEffect::Release("b".into())]);
}
#[test]
fn source_media_click_and_play_button_toggle_and_restart_at_end() {
    let mut state = SourceState::default()
        .plan(SourceCommand::Toggle("a".into()), video)
        .unwrap()
        .next;
    assert!(state.playing);
    let paused = state.plan(SourceCommand::PlayPause, video).unwrap();
    assert_eq!(paused.effects, vec![SourceEffect::Pause]);
    assert!(!paused.next.playing);
    state = paused.next;
    state
        .observe("a", state.epoch, 12.5, Some(12.5), false)
        .unwrap();
    let restarted = state.plan(SourceCommand::PlayPause, video).unwrap();
    assert_eq!(restarted.next.time, 0.0);
    assert_eq!(
        restarted.effects,
        vec![SourceEffect::Seek(0.0), SourceEffect::Play]
    );
}
#[test]
fn source_scrubbing_is_continuous_unquantized_clamped_and_keeps_playing() {
    let state = SourceState::default()
        .plan(SourceCommand::Toggle("a".into()), video)
        .unwrap()
        .next;
    let seek = state
        .plan(SourceCommand::SeekRatio(0.12345), video)
        .unwrap();
    assert_eq!(seek.next.time, 0.12345 * 12.5);
    assert!(seek.next.playing);
    for (ratio, time) in [(-1.0, 0.0), (2.0, 12.5)] {
        assert_eq!(
            state
                .plan(SourceCommand::SeekRatio(ratio), video)
                .unwrap()
                .next
                .time,
            time
        );
    }
    assert!(
        state
            .plan(SourceCommand::SeekRatio(f64::NAN), video)
            .is_err()
    );
}
#[test]
fn source_slider_keys_seek_five_seconds_and_home_end_without_project_frames() {
    let state = SourceState::default()
        .plan(SourceCommand::Show("a".into()), video)
        .unwrap()
        .next;
    let state = state
        .plan(SourceCommand::StepSeconds(5.0), video)
        .unwrap()
        .next;
    assert_eq!(state.time, 5.0);
    assert_eq!(
        state
            .plan(SourceCommand::StepSeconds(-50.0), video)
            .unwrap()
            .next
            .time,
        0.0
    );
    assert_eq!(
        state.plan(SourceCommand::End, video).unwrap().next.time,
        12.5
    );
    assert_eq!(
        state.plan(SourceCommand::Start, video).unwrap().next.time,
        0.0
    );
}
#[test]
fn images_have_no_transport_and_unknown_duration_cannot_scrub() {
    let image = |id: &str| {
        Some(SourceAsset {
            id: id.into(),
            kind: SourceKind::Image,
            available: true,
            duration: None,
        })
    };
    let state = SourceState::default()
        .plan(SourceCommand::Toggle("image".into()), image)
        .unwrap()
        .next;
    assert!(!state.playing);
    assert!(!state.temporal());
    assert!(
        state
            .plan(SourceCommand::PlayPause, video)
            .unwrap()
            .effects
            .is_empty()
    );
    let mut state = SourceState::default()
        .plan(SourceCommand::Show("a".into()), video)
        .unwrap()
        .next;
    state.observe("a", state.epoch, 0.0, None, false).unwrap();
    assert!(state.plan(SourceCommand::SeekRatio(0.5), video).is_err());
}
#[test]
fn missing_or_unavailable_sources_do_not_discard_current_source() {
    let state = SourceState::default()
        .plan(SourceCommand::Show("a".into()), video)
        .unwrap()
        .next;
    assert!(
        state
            .plan(SourceCommand::Show("missing".into()), |_| None)
            .is_err()
    );
    assert!(
        state
            .plan(SourceCommand::Show("bad".into()), |id| Some(SourceAsset {
                available: false,
                ..video(id).unwrap()
            }))
            .is_err()
    );
    assert_eq!(state.active_id(), Some("a"));
}
#[test]
fn stale_and_invalid_source_observations_are_atomic() {
    let mut state = SourceState::default()
        .plan(SourceCommand::Show("a".into()), video)
        .unwrap()
        .next;
    let before = state.clone();
    assert_eq!(
        state.observe("stale", state.epoch, 1.0, Some(2.0), true),
        Err(Error::Stale)
    );
    assert_eq!(state, before);
    assert!(
        state
            .observe("a", state.epoch, f64::NAN, Some(2.0), true)
            .is_err()
    );
    assert_eq!(state, before);
    state
        .observe("a", state.epoch, 99.0, Some(12.5), false)
        .unwrap();
    assert_eq!(state.time, 12.5);
}
#[test]
fn scene_toolbar_modes_axes_view_and_framing_preserve_separate_navigation() {
    let mut state = SceneState {
        transform: TransformMode::Translate,
        axes: AxisSpace::World,
        view: SceneView::Camera,
        navigation: NavigationMode::Orbit,
    };
    for mode in [
        TransformMode::Rotate,
        TransformMode::Scale,
        TransformMode::Translate,
    ] {
        state.apply(&SceneCommand::Transform(mode));
        assert_eq!(state.navigation, NavigationMode::Select);
        assert_eq!(state.transform, mode);
    }
    state.apply(&SceneCommand::Axes(AxisSpace::Local));
    state.apply(&SceneCommand::View(SceneView::Editor));
    state.apply(&SceneCommand::Navigate(NavigationMode::Dolly));
    let before = state.clone();
    state.apply(&SceneCommand::FrameSelection);
    state.apply(&SceneCommand::FrameAll);
    assert_eq!(state, before);
    assert_eq!(state.axes, AxisSpace::Local);
    assert_eq!(state.view, SceneView::Editor);
}
#[test]
fn late_observation_from_previous_load_of_same_asset_is_rejected() {
    let first = SourceState::default()
        .plan(SourceCommand::Show("a".into()), video)
        .unwrap()
        .next;
    let old_epoch = first.epoch;
    let closed = first.plan(SourceCommand::Close, video).unwrap().next;
    let mut reopened = closed
        .plan(SourceCommand::Show("a".into()), video)
        .unwrap()
        .next;
    assert_ne!(reopened.epoch, old_epoch);
    assert_eq!(
        reopened.observe("a", old_epoch, 8.0, Some(12.5), true),
        Err(Error::Stale)
    );
    assert_eq!(reopened.time, 0.0);
    assert!(!reopened.playing);
}
