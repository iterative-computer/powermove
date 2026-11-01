use pm_kernel::state::UiState;
use pm_model::Value;
use pm_panel_viewerchrome::*;
fn transport(ui: &UiState) -> Transport {
    Transport::read(ui, 24.0, 10.0, (1.0, 9.0)).unwrap()
}
#[test]
fn boundaries_and_steps_keep_exact_project_frame_time() {
    let mut ui = UiState::default();
    for (action, expected) in [
        (TransportAction::End, 10.0),
        (TransportAction::Start, 0.0),
        (TransportAction::WorkIn, 1.0),
        (TransportAction::WorkOut, 9.0),
        (TransportAction::Seek(3.1), 74.0 / 24.0),
        (TransportAction::Step(10), 84.0 / 24.0),
        (TransportAction::Step(-1), 83.0 / 24.0),
        (TransportAction::Seek(-10.0), 0.0),
        (TransportAction::Seek(100.0), 10.0),
    ] {
        transport(&ui).apply(action, &mut ui).unwrap();
        assert_eq!(transport(&ui).time, expected);
    }
}
#[test]
fn pause_uses_frame_on_screen_floor_while_seek_uses_nearest_frame() {
    let mut ui = UiState::default();
    let obj = ui.transport.as_object_mut().unwrap();
    obj.insert("time", Value::Number(3.999));
    obj.insert("playing", Value::Bool(true));
    transport(&ui)
        .apply(TransportAction::Pause, &mut ui)
        .unwrap();
    assert_eq!(transport(&ui).time, 95.0 / 24.0);
    assert!(!transport(&ui).playing);
    transport(&ui)
        .apply(TransportAction::Seek(3.999), &mut ui)
        .unwrap();
    assert_eq!(transport(&ui).time, 4.0);
}
#[test]
fn paused_pause_is_noop_and_toggle_retains_valid_time() {
    let mut ui = UiState::default();
    transport(&ui)
        .apply(TransportAction::Pause, &mut ui)
        .unwrap();
    transport(&ui)
        .apply(TransportAction::Toggle, &mut ui)
        .unwrap();
    assert!(transport(&ui).playing);
    transport(&ui)
        .apply(TransportAction::Toggle, &mut ui)
        .unwrap();
    assert!(!transport(&ui).playing);
}
#[test]
fn nonfinite_seek_and_invalid_timing_preserve_valid_state() {
    let mut ui = UiState::default();
    let before = ui.clone();
    assert!(
        transport(&ui)
            .apply(TransportAction::Seek(f64::NAN), &mut ui)
            .is_err()
    );
    assert_eq!(ui, before);
    for fps in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(Transport::read(&ui, fps, 10.0, (0.0, 10.0)).is_err());
    }
    assert!(Transport::read(&ui, 24.0, 10.0, (4.0, 3.0)).is_err());
}
#[test]
fn quality_choice_changes_keep_playback_time_rate_and_opaque_perf_data() {
    let mut ui = UiState {
        transport: pm_model::json::parse(
            r#"{"time":1.2345,"playing":true,"rate":-2,"perf":{"auto":true,"decoder":"preserve"}}"#,
        )
        .unwrap(),
        ..UiState::default()
    };
    for quality in [
        Quality::Quarter,
        Quality::Half,
        Quality::Full,
        Quality::Auto,
    ] {
        Chrome::write_quality(&mut ui, quality).unwrap();
        let chrome = Chrome::read(&ui).unwrap();
        assert_eq!(chrome.quality, quality);
        assert_eq!(chrome.effective_quality, quality.scale());
        assert_eq!(
            ui.transport.get("time").and_then(Value::as_f64),
            Some(1.2345)
        );
        assert_eq!(
            ui.transport.get("playing").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(ui.transport.get("rate").and_then(Value::as_f64), Some(-2.0));
        assert_eq!(
            ui.transport
                .get("perf")
                .unwrap()
                .get("decoder")
                .and_then(Value::as_str),
            Some("preserve")
        );
    }
}
#[test]
fn safe_areas_and_guides_are_explicit_geometry_with_no_invented_presets() {
    let mut ui = UiState::default();
    assert!(Chrome::read(&ui).unwrap().overlays.safe_areas.is_empty());
    ui.viewer.as_object_mut().unwrap().insert("chromeOverlays",pm_model::json::parse(r#"{"guidesVisible":true,"safeAreasVisible":true,"guides":[{"id":"g","axis":"vertical","position":123.456}],"safeAreas":[{"id":"s","left":10,"top":20,"right":1900,"bottom":1000}],"future":42}"#).unwrap());
    let chrome = Chrome::read(&ui).unwrap();
    assert_eq!(chrome.overlays.guides[0].position, 123.456);
    assert_eq!(chrome.overlays.safe_areas[0].left, 10.0);
    assert!(chrome.overlays.safe_areas_visible);
    for invalid in [
        r#"{"guides":[{"id":"g","axis":"vertical"}]}"#,
        r#"{"guides":[{"id":"g","axis":"other","position":1}]}"#,
        r#"{"safeAreas":[{"id":"s","left":10,"top":2,"right":1,"bottom":3}]}"#,
        r#"{"guides":[{"id":"g","axis":"horizontal","position":1},{"id":"g","axis":"horizontal","position":2}]}"#,
    ] {
        ui.viewer
            .as_object_mut()
            .unwrap()
            .insert("chromeOverlays", pm_model::json::parse(invalid).unwrap());
        assert!(Chrome::read(&ui).is_err(), "{invalid}");
    }
}
