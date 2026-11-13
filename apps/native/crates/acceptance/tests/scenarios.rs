use pm_acceptance::{fixture, keyboard, scenarios};
macro_rules! checks {
    ($($name:ident => $id:literal),* $(,)?)=>{$(
        #[test] fn $name() {let s=scenarios::all().into_iter().find(|s|s.id==$id).unwrap();(s.run)().unwrap();}
    )*}
}
checks! {
    effect_validator => "A532",
    wave5_lint => "A539", wave5_simplify => "A529",
    wave5_slip => "B258",
    wave5_strips => "A504", wave5_scene => "A508", wave5_list_media => "A513", wave5_manage_media => "A516", wave5_video => "A530", wave5_captions => "A550", wave5_video_refusals => "W057.mcp-refusals",
    mcp_union => "A494", mcp_timeline => "A503", mcp_manage => "A505", mcp_layout => "A520",
    shuttle => "W079.shuttle", ripple_delete => "W080.ripple-delete", ripple_trim => "W080.ripple-trim", roll => "W081.roll", slide => "W081.slide", source_edit => "W082.source", linked_av => "W083.links",
    composition_settings => "W009.settings", authored_mask => "W048.mask", authored_text => "W049.text", authored_easing => "W061.easing",
    mcp_list => "A495", mcp_create => "A496", mcp_open => "A497",
    mcp_start => "A498", mcp_finish => "A501", mcp_selection => "A517",
    mcp_rollback => "A531", library_lifecycle => "W006.library",
    portable_history => "W007.portable", journal_recovery => "W004.journal",
    atomic_batch => "W030.atomic", interleaved_writers => "W030.interleaved",
    lock_admission => "W030.lock", disconnect_rollback => "W033.disconnect",
    pending_errors => "W033.unavailable", selection_transient => "W046.selection",
    work_area => "W052.work", markers => "W053.markers", strips => "W055.strips",
    split_continuity => "W056.split", source_handles => "W057.slip",
    keyframe_identity => "W060.keys", group_identity => "W062.groups",
    authored_f64 => "W063.properties", actor_history => "W075.history",
    engine_double => "W076.engine-hook", surface_double => "W032.surface-hook",
}
#[test]
fn corpus_json() {
    fixture::corpus_fixture("json").unwrap();
}
#[test]
fn corpus_pmv3() {
    fixture::corpus_fixture("pmv3").unwrap();
}
#[test]
fn corpus_pmv4() {
    fixture::corpus_fixture("pmv4").unwrap();
}
/// Shipped bindings with no v1.2.0 registry row in w1-keymap.md: native
/// additions, pinned here (id, chord, command, arguments as JSON) so none
/// goes unchecked.
const NATIVE_BINDINGS: &[(&str, &str, &str, &str)] = &[
    ("K196", "`", "maximizePanel", "[]"),
    ("K197", "m", "toggleLayerStrips", "[]"),
    ("main.viewer.tab", "tab", "composition.flowchart", "[]"),
    ("native.skimming", "alt+s", "toggleSkimming", "[]"),
    (
        "native.audioSkimming",
        "alt+shift+s",
        "toggleAudioSkimming",
        "[]",
    ),
    ("native.newNull", "cmd+alt+shift+y", "newNull", "[]"),
    ("native.newNull.ctrl", "ctrl+alt+shift+y", "newNull", "[]"),
    // Transport: the shuttle is tried before the keys' shipped meanings.
    ("native.shuttleBackward", "j", "shuttleBackward", "[]"),
    ("native.shuttleStop", "k", "shuttleStop", "[]"),
    ("native.shuttleForward", "l", "shuttleForward", "[]"),
    (
        "native.shuttleBackward.start",
        "ctrl+j",
        "shuttleBackward",
        "[true]",
    ),
    // Editing from the keyboard.
    (
        "native.rippleDelete",
        "shift+backspace",
        "rippleDelete",
        "[]",
    ),
    (
        "native.rippleDelete.forward",
        "shift+delete",
        "rippleDelete",
        "[]",
    ),
    (
        "native.nudgeLayerTime.earlier",
        "alt+pageup",
        "nudgeLayerTime",
        "[-1]",
    ),
    (
        "native.nudgeLayerTime.later",
        "alt+pagedown",
        "nudgeLayerTime",
        "[1]",
    ),
    (
        "native.nudgeLayerTime.earlier10",
        "alt+shift+pageup",
        "nudgeLayerTime",
        "[-10]",
    ),
    (
        "native.nudgeLayerTime.later10",
        "alt+shift+pagedown",
        "nudgeLayerTime",
        "[10]",
    ),
    ("native.addKeyframe", "alt+k", "timeline.addKeyframe", "[]"),
    (
        "native.keyProperty.p",
        "alt+shift+p",
        "timeline.keyProperty:p",
        "[]",
    ),
    (
        "native.keyProperty.s",
        "alt+shift+s",
        "timeline.keyProperty:s",
        "[]",
    ),
    (
        "native.keyProperty.r",
        "alt+shift+r",
        "timeline.keyProperty:r",
        "[]",
    ),
    (
        "native.keyProperty.t",
        "alt+shift+t",
        "timeline.keyProperty:t",
        "[]",
    ),
    (
        "native.keyProperty.a",
        "alt+shift+a",
        "timeline.keyProperty:a",
        "[]",
    ),
    // The timeline's and the viewer's own keys.
    ("native.addMarker", "*", "timeline.addMarker", "[]"),
    (
        "native.addMarker.shift",
        "shift+8",
        "timeline.addMarker",
        "[]",
    ),
    (
        "native.addMarker.ctrl",
        "ctrl+8",
        "timeline.addMarker",
        "[]",
    ),
    ("native.timelineZoomIn", "=", "timeline.zoomIn", "[]"),
    ("native.timelineZoomOut", "-", "timeline.zoomOut", "[]"),
    ("native.timelineFit", "\\", "timeline.fit", "[]"),
    (
        "native.toggleHold",
        "cmd+alt+h",
        "timeline.toggleHold",
        "[]",
    ),
    ("native.safeAreas", "'", "viewer.safeAreas", "[]"),
    ("native.guides", "cmd+;", "viewer.guides", "[]"),
    ("native.showRed", "alt+1", "viewer.showRed", "[]"),
    ("native.showGreen", "alt+2", "viewer.showGreen", "[]"),
    ("native.showBlue", "alt+3", "viewer.showBlue", "[]"),
    ("native.showAlpha", "alt+4", "viewer.showAlpha", "[]"),
    ("native.showAlphaOverlay", "alt+5", "viewer.showAlphaOverlay", "[]"),
    ("native.zoom200", "cmd+2", "viewer.zoom200", "[]"),
    ("native.rulers", "cmd+r", "viewer.rulers", "[]"),
    ("native.grid", "cmd+'", "viewer.grid", "[]"),
    (
        "native.projectSettings",
        "cmd+shift+,",
        "projectSettings",
        "[]",
    ),
    ("native.rippleTrimIn", "alt+shift+[", "rippleTrimIn", "[]"),
    ("native.rippleTrimOut", "alt+shift+]", "rippleTrimOut", "[]"),
    ("native.extendEdit", "shift+x", "extendEdit", "[]"),
    (
        "native.slipLayers.left",
        "cmd+alt+left",
        "slipLayers",
        "[-1]",
    ),
    (
        "native.slipLayers.right",
        "cmd+alt+right",
        "slipLayers",
        "[1]",
    ),
    (
        "native.slipLayers.left10",
        "cmd+alt+shift+left",
        "slipLayers",
        "[-10]",
    ),
    (
        "native.slipLayers.right10",
        "cmd+alt+shift+right",
        "slipLayers",
        "[10]",
    ),
    ("native.slideLayer.left", "alt+,", "slideLayer", "[-1]"),
    ("native.slideLayer.right", "alt+.", "slideLayer", "[1]"),
    (
        "native.slideLayer.left10",
        "alt+shift+,",
        "slideLayer",
        "[-10]",
    ),
    (
        "native.slideLayer.right10",
        "alt+shift+.",
        "slideLayer",
        "[10]",
    ),
    ("native.timeRemap", "cmd+alt+t", "timeRemapLayers", "[]"),
    ("native.timeReverse", "cmd+alt+r", "reverseLayers", "[]"),
    ("native.freezeFrame", "alt+f", "freezeFrame", "[]"),
];
#[test]
fn every_shipped_binding_under_seven_routing_contexts() {
    let c = pm_acceptance::catalog::Catalog::embedded().unwrap();
    let shipped = keyboard::production::shipped();
    assert_eq!(shipped.len(), 250);
    assert_eq!(c.bindings.len() + NATIVE_BINDINGS.len(), shipped.len());
    for real in &shipped {
        let binding = match c.bindings.iter().find(|b| b.id == real.id) {
            Some(b) => b.clone(),
            None => {
                let (id, chord, command, args) = *NATIVE_BINDINGS
                    .iter()
                    .find(|n| n.0 == real.id)
                    .unwrap_or_else(|| panic!("{} is neither in the catalog nor pinned", real.id));
                pm_acceptance::catalog::Binding {
                    id: id.into(),
                    chord: chord.into(),
                    command: command.into(),
                    args: serde_json::from_str(args).unwrap(),
                    priority: i64::from(real.priority),
                    in_fields: real.in_fields,
                    repeat: real.repeat,
                    loose: real.loose,
                    workflow: String::new(),
                    source: String::new(),
                }
            }
        };
        for context in keyboard::CONTEXTS {
            keyboard::check(&binding, context).unwrap();
        }
    }
    // Every pinned row is a shipped one: a row removed from the keymap
    // leaves the list too.
    for (id, ..) in NATIVE_BINDINGS {
        assert!(shipped.iter().any(|b| b.id == *id), "{id} is not shipped");
    }
}
#[test]
fn corpus_copy_refuses_symlinks_and_never_overwrites_source() {
    let source = fixture::Scratch::new().unwrap();
    let dest = fixture::Scratch::new().unwrap();
    let path = source.0.join("synthetic.pmv");
    std::fs::write(&path, b"not a project").unwrap();
    assert!(scenarios::corpus_copy(&path, 0).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"not a project");
    let copy = dest.copy(&path, 0).unwrap();
    assert_ne!(copy, path);
    assert!(dest.copy(&path, 0).is_err());
    #[cfg(unix)]
    {
        let link = source.0.join("link.pmv");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(dest.copy(&link, 1).is_err());
    }
}

/// The merged kernel fix must keep the exact authored rate through persistence.
#[test]
fn fractional_fps_stays_exact_now_that_the_kernel_keeps_authored_rates() {
    let scenario = scenarios::all()
        .into_iter()
        .find(|s| s.id == "W071.fractional-rate")
        .unwrap();
    let result = pm_acceptance::runner::execute(
        scenario.id,
        scenario.workflow,
        "supporting-probe",
        scenario.title,
        scenario.run,
    );
    assert_eq!(
        result.status,
        pm_acceptance::runner::Status::Pass,
        "{}",
        result.reason
    );
}

#[test]
fn frozen_replace_and_sequential_set_preserve_complete_authored_curves() {
    pm_acceptance::curve_oracle::replace().unwrap();
    pm_acceptance::curve_oracle::set().unwrap();
}
