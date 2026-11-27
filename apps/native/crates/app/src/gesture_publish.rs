//! Which built-in panels republish while a pointer gesture is under way
//! (ui-latency N2, Opus ruling 2026-11-21).
//!
//! A drag or a scrub (a viewer handle, a Properties number field, a timeline
//! clip) holds a kernel edit open, and every step is a publication. During
//! it only the panels that are **on screen** and **show the edited layers**
//! republish live; hidden or unrelated panels keep their last tree and
//! update once, at the gesture's end, with the commit's own publication.
//! After the release every panel is consistent within that one publication.
//!
//! Not gestures, so they publish as always: undo and redo, arrow-key nudges
//! and MCP or agent edits (no edit held open under a held pointer button),
//! text typed in the viewer (an open edit, but no button held), and an MCP
//! synthetic drag (no OS button is down for it).
use std::collections::BTreeSet;

/// The shipped built-in panels whose content follows the layers a gesture
/// edits: the timeline's rows and keys, Properties, Effect Controls, and
/// Color and Easing (the selection's colour and keys). The others (Project,
/// Effects, the viewer's bar, Agent, Settings, Export) show nothing a drag
/// step changes. Forks and extensions always publish (`publish_live`).
pub(crate) const FOLLOWS_GESTURE: [&str; 5] =
    ["timeline", "inspector", "layer-effects", "color", "easing"];

/// What the next built-in publish covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Every realm (as always).
    All,
    /// A gesture is under way: only these built-ins (plus forks and
    /// extensions); the others wait for its end.
    Live(BTreeSet<String>),
}

/// The kernel holds an edit open while a pointer button is down: a drag or a
/// scrub, not typing, a hold left open, or a synthetic (MCP) drag.
pub(crate) fn gesture_open(entry_open: bool, pointer_held: impl FnOnce() -> bool) -> bool {
    entry_open && pointer_held()
}

/// A mouse or trackpad button is down (AppKit's own state, not GPUI's: an
/// MCP synthetic drag holds none).
pub(crate) fn pointer_held() -> bool {
    #[cfg(test)]
    if let Some(held) = POINTER_HELD.with(std::cell::Cell::get) {
        return held;
    }
    objc2_app_kit::NSEvent::pressedMouseButtons() != 0
}
#[cfg(test)]
thread_local! {
    /// Tests stand in for the pointer (`None`: ask AppKit).
    pub(crate) static POINTER_HELD: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// The panels on screen (`Integration::shown_panels`), kept only when they
/// change: the shell reports them on every frame it draws.
pub(crate) fn follow_shown(kept: &mut BTreeSet<String>, shown: &[String]) {
    if shown.len() != kept.len() || shown.iter().any(|id| !kept.contains(id)) {
        *kept = shown.iter().cloned().collect();
    }
}

/// Whether some built-in panel waited during a gesture and is still owed its
/// publication.
#[derive(Debug, Default)]
pub(crate) struct Deferral {
    deferred: bool,
    /// A watch for a release that brings no publication is running.
    pub(crate) watching: bool,
}

impl Deferral {
    /// The scope of the publish about to start. Outside a gesture: every
    /// realm, which also settles every panel that waited.
    pub(crate) fn scope(&mut self, gesture: bool, shown: &BTreeSet<String>) -> Scope {
        if !gesture {
            self.deferred = false;
            return Scope::All;
        }
        self.deferred = true;
        Scope::Live(
            FOLLOWS_GESTURE
                .iter()
                .filter(|id| shown.contains(**id))
                .map(|id| (*id).to_owned())
                .collect(),
        )
    }
    /// The gesture is over but a panel still waits (its end brought no
    /// publication of its own): publish every realm now.
    pub(crate) fn owed(&self, gesture: bool) -> bool {
        self.deferred && !gesture
    }
    pub(crate) fn deferred(&self) -> bool {
        self.deferred
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_builtins::{Builtins, Publication};
    use pm_kernel::commands::{Origin, properties::SetProperty};
    use pm_kernel::{
        Call, Caller, GestureCall, HistoryCall, KernelActor, KernelConfig, KernelHandle,
        OpenSource, PropertyCommand, Reply, Request, SelectionCall, WriteBody,
    };
    use pm_runtime_v8::{Evaluation, Ticket};
    use serde_json::Value;
    use std::time::{Duration, Instant};

    mod drag_fixture {
        include!("../../kernel/src/drag_fixture.rs");
    }

    #[test]
    fn layer_drag_profile_panel_workers() {
        let temp = tempfile::tempdir().unwrap();
        let (_actor, kernel) = drag_fixture::open(temp.path().join("journals"));
        let mut panels = Builtins::new(kernel.clone(), drag_fixture::PROJECT, || {}).unwrap();
        for id in ["inspector", "layer-effects"] {
            finish(panels.mount(id).unwrap());
        }
        panels.update_facts(serde_json::json!({"time": drag_fixture::TIME}));

        // Anchor Point runs on the separate compatibility worker. A local
        // profile copies its code and adds a completion marker to its panel
        // listener. The marker is stored only in the temporary kernel.
        let anchor = std::env::var_os("PM_PROFILE_ANCHOR_POINT").map(|installed| {
            use pm_compat_runner::host::{ExtensionHost, HostConfig, settle_document};
            let installed = std::path::PathBuf::from(installed);
            let roots = temp.path().join("extensions");
            let copied = roots.join("anchor-point");
            std::fs::create_dir_all(&copied).unwrap();
            std::fs::copy(installed.join("manifest.json"), copied.join("manifest.json")).unwrap();
            let source = std::fs::read_to_string(installed.join("index.ts")).unwrap();
            let instrumented = source
                .replacen("const listeners=[", "let dragProbe = 0; api.storage.set('dragProbe', dragProbe); const listeners=[", 1)
                .replacen("api.on('project:changed',update)",
                    "api.on('project:changed',()=>{update(); api.storage.set('dragProbe', ++dragProbe)})", 1);
            assert_ne!(source, instrumented, "Anchor Point source changed");
            std::fs::write(copied.join("index.ts"), instrumented).unwrap();
            let host = ExtensionHost::start(HostConfig {
                kernel: kernel.clone(),
                project: drag_fixture::PROJECT.into(),
                roots: vec![roots],
                cache: temp.path().join("extension-cache"),
                owner: None,
                text: None,
                frames: None,
                rasters: None,
                vars: None,
                clipboard: None,
                wake: std::sync::Arc::new(|| {}),
            });
            let initial = kernel.published(drag_fixture::PROJECT).unwrap();
            settle_document(initial.snapshot.doc_id, Duration::from_secs(20), &|| false)
                .unwrap().expect("Anchor Point host");
            host.set_window_shown(true);
            host.set_viewport(1280., 720., 1.);
            host.set_panel("anchor-point", true);
            host.place_panel("anchor-point", Some([0., 0., 220., 300.]));
            wait_for_anchor_turn(&kernel, &host, 0);
            host
        });

        let mut builtins = Vec::new();
        let mut legacy = Vec::new();
        drag_fixture::drag(&kernel, |_| {
            let start = Instant::now();
            let (tickets, _) = panels.publish_live(|id| matches!(id, "inspector" | "layer-effects"));
            let dispatched = start.elapsed().as_secs_f64() * 1000.;
            let after_dispatch = Instant::now();
            if let Some(anchor) = &anchor {
                wait_for_anchor_turn(&kernel, anchor, legacy.len() as u64 + 1);
                legacy.push([start.elapsed().as_secs_f64() * 1000.]);
            }
            for (_, ticket) in tickets {
                finish(ticket.unwrap());
            }
            builtins.push([dispatched, after_dispatch.elapsed().as_secs_f64() * 1000.]);
        });
        drag_fixture::report(["V8 dispatch", "V8 panel worker round trip"], &builtins);
        if anchor.is_some() {
            drag_fixture::report(["Anchor Point compatibility worker, concurrent with V8"], &legacy);
        }
        drag_fixture::assert_budget(&builtins, [0]);
        panels.close();
    }

    fn wait_for_anchor_turn(
        kernel: &KernelHandle,
        host: &pm_compat_runner::host::ExtensionHost,
        turn: u64,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let storage = pm_compat_runner::adapter::storage_of(kernel);
            if storage.as_object().unwrap().iter().any(|(key, value)| {
                key.ends_with(".dragProbe") && value.as_u64().is_some_and(|value| value >= turn)
                    || value["dragProbe"].as_u64().is_some_and(|value| value >= turn)
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "Anchor Point missed turn {turn}: {:?}", host.records());
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    const PROJECT: &str = "n2";
    /// Mounted for these tests: a relevant panel shown, a relevant panel
    /// hidden, an unrelated panel shown and an unrelated panel hidden.
    const MOUNTED: [&str; 4] = ["inspector", "layer-effects", "assets", "fxbrowser"];

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn during_a_gesture_only_shown_panels_that_show_the_layers_publish() {
        let shown = set(&["inspector", "timeline", "assets", "viewer", "agent"]);
        let mut d = Deferral::default();
        assert_eq!(
            d.scope(true, &shown),
            Scope::Live(set(&["timeline", "inspector"])),
            "hidden Effect Controls, Color and Easing wait; shown Project, viewer bar and Agent are unrelated"
        );
        assert!(d.deferred());
        assert!(!d.owed(true), "still dragging");
        assert!(d.owed(false), "released with no publication of its own");
        assert_eq!(d.scope(false, &shown), Scope::All);
        assert!(
            !d.deferred() && !d.owed(false),
            "the full publish settles them"
        );
        // Nothing relevant on screen: every built-in waits.
        assert_eq!(d.scope(true, &set(&["assets"])), Scope::Live(set(&[])));
    }

    #[test]
    fn only_an_edit_held_open_under_a_held_button_is_a_gesture() {
        // A drag or a scrub.
        assert!(gesture_open(true, || true));
        // Typing in the viewer, an edit left open, an MCP synthetic drag.
        assert!(!gesture_open(true, || false));
        // Undo, redo, nudges, MCP and agent edits hold no edit open, even
        // with a button down elsewhere; the pointer is not even asked.
        assert!(!gesture_open(false, || panic!(
            "asked the pointer without an open edit"
        )));
        let mut d = Deferral::default();
        assert_eq!(
            d.scope(gesture_open(false, || true), &set(&["inspector"])),
            Scope::All
        );
    }

    #[test]
    fn shown_panels_are_kept_only_when_they_change() {
        let mut kept = BTreeSet::new();
        follow_shown(&mut kept, &["inspector".into(), "viewer".into()]);
        assert_eq!(kept, set(&["inspector", "viewer"]));
        follow_shown(&mut kept, &["viewer".into(), "inspector".into()]);
        assert_eq!(kept, set(&["inspector", "viewer"]));
        follow_shown(&mut kept, &["viewer".into()]);
        assert_eq!(kept, set(&["viewer"]));
    }

    #[test]
    fn pointer_stands_in_for_tests() {
        POINTER_HELD.with(|p| p.set(Some(true)));
        assert!(pointer_held());
        POINTER_HELD.with(|p| p.set(Some(false)));
        assert!(!pointer_held());
        POINTER_HELD.with(|p| p.set(None));
    }

    // ---------------------------------------------- real kernel, real realms

    struct Stage {
        /// What each realm published when it mounted: what its view shows
        /// until it publishes again.
        mounted: Vec<Publication>,
        kernel: KernelHandle,
        panels: Builtins,
        _actor: KernelActor,
    }
    impl Drop for Stage {
        fn drop(&mut self) {
            self.panels.close();
        }
    }

    fn finish(ticket: Ticket<Evaluation>) {
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            if let Some(result) = ticket.try_take() {
                result.expect("panel evaluation");
                return;
            }
            assert!(Instant::now() < deadline, "panel ticket timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn call(kernel: &KernelHandle, caller: Caller, call: Call) -> Reply {
        kernel
            .call(PROJECT, Request::new(caller, call))
            .wait()
            .expect("kernel reply")
            .expect("kernel accepted")
            .reply
    }

    fn opacity(value: f64) -> WriteBody {
        WriteBody::Commands(vec![
            PropertyCommand::Set(SetProperty::new("a", "opacity", value, 0.)).into(),
        ])
    }

    fn stage() -> Stage {
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        let project = pm_model::json::parse(r##"{"id":"n2","name":"Gesture","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"a","type":"shape","name":"Headline","from":0,"dur":10,"on":true,"lock":false,"p":{"opacity":{"v":100,"kf":[]},"position.x":{"v":320,"kf":[]},"position.y":{"v":180,"kf":[]}},"d":{"w":100,"h":100},"fx":[]}]}"##).unwrap();
        kernel
            .open(PROJECT, OpenSource::Untitled(project))
            .wait()
            .unwrap()
            .unwrap();
        call(
            &kernel,
            Caller::native(),
            SelectionCall::Layers {
                ids: vec!["a".into()],
                add: false,
            }
            .into(),
        );
        let mut panels = Builtins::new(kernel.clone(), PROJECT, || {}).unwrap();
        for id in MOUNTED {
            finish(panels.mount(id).unwrap());
        }
        let mounted = panels.take_publications();
        Stage {
            mounted,
            kernel,
            panels,
            _actor: actor,
        }
    }

    /// Publishes as `update_builtins` does for `scope`; returns which
    /// realms were asked, once every one has answered.
    fn publish(s: &Stage, scope: &Scope) -> BTreeSet<String> {
        let tickets = match scope {
            Scope::All => s.panels.publish(),
            Scope::Live(live) => {
                let (tickets, waited) = s.panels.publish_live(|id| live.contains(id));
                assert_eq!(
                    waited,
                    MOUNTED.iter().any(|id| !live.contains(*id)),
                    "says whether any panel waited"
                );
                tickets
            }
        };
        tickets
            .into_iter()
            .map(|(id, ticket)| {
                finish(ticket.unwrap());
                id
            })
            .collect()
    }

    /// Properties' Opacity field in `p`, as its props' JSON.
    fn shown_opacity(p: &[Publication]) -> Option<String> {
        p.iter()
            .filter(|p| p.panel == "inspector")
            .last()
            .and_then(|p| {
                p.surface
                    .nodes()
                    .values()
                    .find(|n| n.props.get("id").and_then(Value::as_str) == Some("value:opacity:0"))
                    .map(|n| Value::Object(n.props.clone()).to_string())
            })
    }

    fn entry_open(s: &Stage) -> bool {
        s.kernel.published(PROJECT).unwrap().entry.is_some()
    }

    fn panels(ids: &[Publication]) -> BTreeSet<String> {
        ids.iter().map(|p| p.panel.clone()).collect()
    }

    #[test]
    fn a_gesture_publishes_shown_layer_panels_live_and_every_panel_at_its_end() {
        let s = stage();
        let mut d = Deferral::default();
        let Reply::Gesture(gesture) = call(
            &s.kernel,
            Caller::native(),
            Call::Gesture(GestureCall::Begin {
                label: "Change Property".into(),
            }),
        ) else {
            panic!("no gesture");
        };

        // Step 1, Properties on screen with Project; Effect Controls and
        // Effects hidden: Properties alone shows the step, live.
        call(
            &s.kernel,
            Caller::native(),
            Call::Gesture(GestureCall::Step {
                gesture,
                body: opacity(37.),
                replace: true,
            }),
        );
        let scope = d.scope(
            gesture_open(entry_open(&s), || true),
            &set(&["inspector", "assets", "viewer"]),
        );
        assert_eq!(scope, Scope::Live(set(&["inspector"])));
        assert_eq!(
            publish(&s, &scope),
            set(&["inspector"]),
            "hidden and unrelated panels skipped"
        );
        let live = s.panels.take_publications();
        assert!(panels(&live).contains("inspector"));
        assert!(
            !panels(&live).contains("layer-effects"),
            "hidden Effect Controls waits"
        );
        let shown = shown_opacity(&live).expect("Properties' Opacity field");
        assert!(shown.contains("\"value\":37"), "the step is live: {shown}");

        // Step 2 with Properties now hidden (another tab in front): it waits too.
        call(
            &s.kernel,
            Caller::native(),
            Call::Gesture(GestureCall::Step {
                gesture,
                body: opacity(55.),
                replace: true,
            }),
        );
        let scope = d.scope(gesture_open(entry_open(&s), || true), &set(&["assets"]));
        assert_eq!(publish(&s, &scope), set(&[]));
        assert!(
            shown_opacity(&s.panels.take_publications()).is_none(),
            "hidden Properties keeps its last tree mid-gesture"
        );

        // Release: the commit's own publication publishes every realm, so the
        // panels that waited show the committed value in that one publication.
        call(
            &s.kernel,
            Caller::native(),
            Call::Gesture(GestureCall::Commit { gesture }),
        );
        let scope = d.scope(gesture_open(entry_open(&s), || false), &set(&["assets"]));
        assert_eq!(scope, Scope::All);
        assert!(!d.owed(false));
        assert_eq!(
            publish(&s, &scope),
            set(&MOUNTED),
            "every realm at the commit"
        );
        let settled = s.panels.take_publications();
        let shown = shown_opacity(&settled).expect("Properties published at the commit");
        assert!(
            shown.contains("\"value\":55"),
            "consistent at release: {shown}"
        );
        assert!(s.panels.take_errors().is_empty());
    }

    #[test]
    fn a_cancelled_gesture_settles_every_panel_on_the_value_before_it() {
        let s = stage();
        let mut d = Deferral::default();
        let Reply::Gesture(gesture) = call(
            &s.kernel,
            Caller::native(),
            Call::Gesture(GestureCall::Begin {
                label: "Change Property".into(),
            }),
        ) else {
            panic!("no gesture");
        };
        call(
            &s.kernel,
            Caller::native(),
            Call::Gesture(GestureCall::Step {
                gesture,
                body: opacity(12.),
                replace: true,
            }),
        );
        // Nothing relevant shown: every built-in waits.
        let scope = d.scope(gesture_open(entry_open(&s), || true), &set(&["assets"]));
        assert_eq!(publish(&s, &scope), set(&[]));
        call(
            &s.kernel,
            Caller::native(),
            Call::Gesture(GestureCall::Cancel { gesture }),
        );
        // A release with no publication of its own is still settled at once.
        assert!(d.owed(gesture_open(entry_open(&s), || false)));
        let scope = d.scope(gesture_open(entry_open(&s), || false), &set(&["assets"]));
        assert_eq!(publish(&s, &scope), set(&MOUNTED));
        // Properties never showed the step (it waited), so its tree after
        // the cancel is the one it shows already: that commit changes
        // nothing and is not published again (PM-698). Its view shows its
        // newest publication either way.
        let mut seen = s.mounted.clone();
        seen.extend(s.panels.take_publications());
        let shown = shown_opacity(&seen).expect("Properties");
        assert!(
            shown.contains("\"value\":100"),
            "back to the value before: {shown}"
        );
    }

    #[test]
    fn undo_redo_nudges_and_mcp_edits_publish_every_panel_as_before() {
        let s = stage();
        let mut d = Deferral::default();
        let shown = set(&["inspector"]);
        // Each with a button held somewhere: still no open edit, so all.
        let every = |s: &Stage, d: &mut Deferral, what: &str| {
            assert!(!entry_open(s), "{what} holds no edit open");
            let scope = d.scope(gesture_open(entry_open(s), || true), &shown);
            assert_eq!(scope, Scope::All, "{what}");
            assert_eq!(publish(s, &scope), set(&MOUNTED), "{what}");
            s.panels.take_publications()
        };
        // An MCP edit.
        call(
            &s.kernel,
            Caller::realm(pm_kernel::tx::NATIVE_REALM, 1, Origin::Mcp),
            Call::Transact {
                label: Some("Opacity".into()),
                body: opacity(40.),
                expected_revision: None,
            },
        );
        let p = every(&s, &mut d, "an MCP edit");
        assert!(shown_opacity(&p).is_some_and(|o| o.contains("\"value\":40")));
        // A nudge: one native transaction per key.
        call(
            &s.kernel,
            Caller::native(),
            Call::Transact {
                label: Some("Nudge Selection".into()),
                body: WriteBody::Commands(vec![
                    PropertyCommand::Set(SetProperty::new("a", "position.x", 321., 0.)).into(),
                ]),
                expected_revision: None,
            },
        );
        every(&s, &mut d, "a nudge");
        call(
            &s.kernel,
            Caller::native(),
            Call::History(HistoryCall::Undo),
        );
        every(&s, &mut d, "undo");
        call(
            &s.kernel,
            Caller::native(),
            Call::History(HistoryCall::Redo),
        );
        let p = every(&s, &mut d, "redo");
        assert!(shown_opacity(&p).is_some_and(|o| o.contains("\"value\":40")));
        // An MCP synthetic drag: an open edit, but no OS button down.
        let Reply::Gesture(gesture) = call(
            &s.kernel,
            Caller::native(),
            Call::Gesture(GestureCall::Begin {
                label: "Move".into(),
            }),
        ) else {
            panic!("no gesture");
        };
        call(
            &s.kernel,
            Caller::native(),
            Call::Gesture(GestureCall::Step {
                gesture,
                body: opacity(66.),
                replace: true,
            }),
        );
        let scope = d.scope(gesture_open(entry_open(&s), || false), &shown);
        assert_eq!(scope, Scope::All, "a synthetic drag publishes as before");
        assert_eq!(publish(&s, &scope), set(&MOUNTED));
        call(
            &s.kernel,
            Caller::native(),
            Call::Gesture(GestureCall::Commit { gesture }),
        );
        assert!(s.panels.take_errors().is_empty());
    }
}
