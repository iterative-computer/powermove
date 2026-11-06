//! Host tests: real GPUI input events through the real v4 host, V8 panels and
//! kernel. AppKit slots are absent in the test window; every other path is live.
use crate::v4_view::{Natives, SurfaceView};
use gpui::{
    AppContext, Bounds, Context, Entity, IntoElement, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, ParentElement, PinchEvent, Pixels, Render, ScrollDelta, ScrollWheelEvent,
    StyleRefinement, Styled, TestAppContext, VisualTestContext, Window, div, point, px, size,
};
use pm_builtins::{Builtins, Publication};
use pm_kernel::{KernelActor, KernelConfig, KernelHandle, OpenSource};
use pm_surface::{Commit, Op, Surface, kit::Component};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration, time::Instant};

pub(crate) const PROJECT: &str = "host";
pub(crate) fn project_json(layers: usize) -> String {
    let rows = (0..layers)
        .map(|i| {
            format!(
                r##"{{"id":"l{i}","type":"shape","name":"Layer {i}","solo":{solo},"from":{from},"dur":2,"on":true,"p":{{"opacity":{{"v":100,"kf":[]}},"x":{{"v":10,"kf":[]}},"y":{{"v":20,"kf":[]}},"sx":{{"v":100,"kf":[]}},"sy":{{"v":100,"kf":[]}},"r":{{"v":0,"kf":[]}}}},"d":{{"w":100,"h":100}},"fx":[]}}"##,
                from = i as f64 * 0.5,
                solo = i == 0
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r##"{{"id":"integration","name":"Synthetic Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{{}},"comps":{{}},"layers":[{rows}]}}"##
    )
}
pub(crate) struct Rig {
    pub kernel: KernelHandle,
    pub runtime: Arc<Builtins>,
    _actor: KernelActor,
}
/// Three adjacent static strips on one lane, with handles: pro-edit material.
pub(crate) fn strips_json() -> String {
    let strip = |id: &str, from: f64| {
        format!(
            r##"{{"id":"{id}","type":"video","name":"{id}","from":{from},"dur":2,"track":0,"on":true,"d":{{"asset":"source","trim":{{"v":{trim},"kf":[],"expr":null}},"speed":1}},"p":{{}},"fx":[]}}"##,
            trim = from + 2.
        )
    };
    format!(
        r##"{{"id":"integration","name":"Strips","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{{"source":{{"id":"source","kind":"video","dur":100}}}},"comps":{{}},"layers":[{},{},{}]}}"##,
        strip("a", 0.),
        strip("b", 2.),
        strip("c", 4.)
    )
}
pub(crate) fn rig(panel: &str, layers: usize) -> Rig {
    rig_project(panel, &project_json(layers))
}
pub(crate) fn rig_project(panel: &str, project: &str) -> Rig {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    let project = pm_model::json::parse(project).unwrap();
    kernel
        .open(PROJECT, OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let mut runtime = Builtins::new(kernel.clone(), PROJECT, || {}).unwrap();
    let ticket = runtime.mount(panel).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = ticket.try_take() {
            result.expect("built-in mount");
            break;
        }
        assert!(Instant::now() < deadline, "panel did not mount");
        std::thread::sleep(Duration::from_millis(1));
    }
    Rig {
        kernel,
        runtime: Arc::new(runtime),
        _actor: actor,
    }
}
pub(crate) fn publication(rig: &Rig) -> Publication {
    rig.runtime
        .take_publications()
        .pop()
        .expect("accepted surface")
}
/// Lets real worker threads and the deterministic GPUI queue make progress.
pub(crate) fn settle(cx: &mut TestAppContext, until: impl Fn(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if until(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "input did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
}
pub(crate) fn tokens() -> pm_design::Tokens {
    pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default())
}
pub(crate) fn window(
    cx: &mut TestAppContext,
    rig: &Rig,
    first: Publication,
    natives: Natives,
) -> (Entity<SurfaceView>, VisualTestContext) {
    // Kernel and V8 workers are real threads: their wakeups are expected here.
    cx.executor().allow_parking();
    let runtime = rig.runtime.clone();
    let (view, vcx) = cx.add_window_view(|window, cx| {
        SurfaceView::new(runtime, first, tokens(), natives, window, cx)
    });
    let owned = vcx.clone();
    owned.simulate_resize(size(px(1000.), px(600.)));
    (view, owned)
}
pub(crate) fn bounds(vcx: &mut VisualTestContext, name: &str) -> Bounds<Pixels> {
    vcx.debug_bounds(Box::leak(name.to_owned().into_boxed_str()))
        .unwrap_or_else(|| panic!("{name} was not painted"))
}
fn node_ops(id: u64, parent: u64, kind: &str, props: Value, events: &[&str]) -> Vec<Op> {
    vec![
        Op::Create {
            id,
            r#type: kind.into(),
        },
        Op::InsertBefore {
            parent,
            id,
            before: None,
        },
        Op::SetProps {
            id,
            patch: props.as_object().unwrap().clone(),
        },
        Op::SetListeners {
            id,
            flags: events
                .iter()
                .map(|e| {
                    (
                        (*e).to_owned(),
                        pm_surface::Listener {
                            bubble: true,
                            ..Default::default()
                        },
                    )
                })
                .collect(),
        },
    ]
}
/// Host-only publication: exercises delivery without a panel's JS opinion.
fn synthetic(_: &Rig, ops: Vec<Op>) -> Publication {
    let mut surface = Surface::default();
    surface
        .apply(Commit {
            base_revision: 0,
            ops,
        })
        .unwrap();
    Publication {
        panel: "timeline".into(),
        generation: 1,
        surface: Arc::new(surface),
    }
}
fn events(view: &Entity<SurfaceView>, vcx: &mut VisualTestContext) -> Vec<(u64, String)> {
    view.update(vcx, |v, _| {
        v.log.drain(..).map(|(id, e, _)| (id, e)).collect()
    })
}
fn modifiers_of(shift: bool) -> Modifiers {
    Modifiers {
        shift,
        ..Default::default()
    }
}
#[gpui::test]
fn press_drag_release_stays_on_the_captured_node_outside_its_bounds(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let mut ops = node_ops(1, 0, "div", json!({"style":{"width":100,"height":40}}), &[]);
    ops.extend(node_ops(
        2,
        1,
        "div",
        json!({"id":"target","style":{"width":60,"height":20}}),
        &["pointerdown", "pointermove", "pointerup"],
    ));
    let (view, mut vcx) = window(cx, &rig, synthetic(&rig, ops), Default::default());
    vcx.run_until_parked();
    let at = bounds(&mut vcx, "target").center();
    vcx.simulate_mouse_down(at, MouseButton::Left, modifiers_of(true));
    vcx.simulate_mouse_move(
        point(px(900.), px(500.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.simulate_mouse_up(
        point(px(900.), px(500.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.simulate_mouse_move(point(px(10.), px(10.)), None, Modifiers::none());
    vcx.run_until_parked();
    let log = view.update(&mut vcx, |v, _| std::mem::take(&mut v.log));
    assert_eq!(
        log.iter()
            .map(|(id, e, _)| (*id, e.as_str()))
            .collect::<Vec<_>>(),
        vec![(2, "pointerdown"), (2, "pointermove"), (2, "pointerup")]
    );
    assert_eq!(log[0].2.shift_key, Some(true));
    assert_eq!(log[0].2.button, Some(0));
    assert_eq!(log[1].2.x, Some(900.));
    assert_eq!(log[1].2.buttons, Some(1));
    assert_eq!(log[2].2.buttons, Some(0));
}
#[gpui::test]
fn double_click_right_click_hover_wheel_and_pinch_reach_their_listeners(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let ops = node_ops(
        1,
        0,
        "div",
        json!({"id":"target","style":{"width":200,"height":100}}),
        &[
            "click",
            "doubleclick",
            "contextmenu",
            "pointerenter",
            "pointerleave",
            "wheel",
        ],
    );
    let (view, mut vcx) = window(cx, &rig, synthetic(&rig, ops), Default::default());
    vcx.run_until_parked();
    let at = bounds(&mut vcx, "target").center();
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        events(&view, &mut vcx),
        vec![(1, "pointerenter".to_owned())]
    );
    vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Right,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Right,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 1,
    });
    assert_eq!(events(&view, &mut vcx), vec![(1, "contextmenu".to_owned())]);
    for clicks in [1, 2] {
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: clicks,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: clicks,
        });
    }
    vcx.run_until_parked();
    assert_eq!(
        events(&view, &mut vcx),
        vec![
            (1, "click".to_owned()),
            (1, "click".to_owned()),
            (1, "doubleclick".to_owned())
        ]
    );
    vcx.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(3.), px(7.))),
        ..Default::default()
    });
    vcx.simulate_event(PinchEvent {
        position: at,
        delta: 0.5,
        ..Default::default()
    });
    vcx.run_until_parked();
    let log = view.update(&mut vcx, |v, _| std::mem::take(&mut v.log));
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].2.delta_y, Some(-7.));
    assert_eq!(log[1].2.ctrl_key, Some(true));
    assert_eq!(log[1].2.delta_y, Some(-50.));
    vcx.simulate_mouse_move(point(px(900.), px(500.)), None, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        events(&view, &mut vcx),
        vec![(1, "pointerleave".to_owned())]
    );
}
#[gpui::test]
fn every_kit_component_renders_a_real_element(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let mut ops = node_ops(
        1,
        0,
        "div",
        json!({"style":{"width":600,"height":560}}),
        &[],
    );
    let names: Vec<String> = Component::catalog()
        .map(|c| {
            serde_json::to_value(c)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(names.len(), 29);
    for (i, name) in names.iter().enumerate() {
        let primitive = match name.as_str() {
            "sheet-anchor" | "popover-anchor" | "colour-popover" => "anchored",
            "number-field" | "text-field" => "input",
            _ => "div",
        };
        let mut props = json!({"id":name,"data-pm-component":name,"label":name,"style":{"height":16,"width":20}});
        match name.as_str() {
            "number-field" => props["control"] = json!("number"),
            "text-field" => props["control"] = json!("text"),
            "progress-row" => props["aria-valuenow"] = json!(0.5),
            "colour-well" => props["value"] = json!("#ff0000"),
            "viewer-gizmo" => {
                props["metadata"] = json!({"pmKit":{"kind":"transform","position":{"x":4.,"y":5.}}})
            }
            _ => {}
        }
        ops.extend(node_ops(2 + i as u64, 1, primitive, props, &[]));
    }
    let (view, mut vcx) = window(cx, &rig, synthetic(&rig, ops), Default::default());
    vcx.run_until_parked();
    for name in names {
        assert!(
            vcx.debug_bounds(Box::leak(name.clone().into_boxed_str()))
                .is_some(),
            "{name} rendered no element"
        );
    }
    assert!(view.read_with(&vcx, |v, _| v.renders) > 0);
}
#[gpui::test]
fn cached_panel_does_not_rerender_for_a_parent_frame(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let ops = node_ops(1, 0, "div", json!({"style":{"width":60,"height":60}}), &[]);
    let surface = synthetic(&rig, ops);
    let runtime = rig.runtime.clone();
    let (root, vcx) = cx.add_window_view(|window, cx| {
        let child = cx
            .new(|cx| SurfaceView::new(runtime, surface, tokens(), Default::default(), window, cx));
        Host { child }
    });
    let mut vcx = vcx.clone();
    vcx.run_until_parked();
    let child = root.read_with(&vcx, |r, _| r.child.clone());
    let first = child.read_with(&vcx, |v, _| v.renders);
    assert!(first > 0);
    for _ in 0..8 {
        root.update(&mut vcx, |_, cx| cx.notify());
        vcx.run_until_parked();
    }
    assert_eq!(child.read_with(&vcx, |v, _| v.renders), first);
    child.update(&mut vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert_eq!(child.read_with(&vcx, |v, _| v.renders), first + 1);
}
struct Host {
    child: Entity<SurfaceView>,
}
impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            self.child
                .clone()
                .cached(StyleRefinement::default().size_full()),
        )
    }
}
/// A real Timeline click travels GPUI -> host -> V8 -> kernel; undo reverts it.
#[gpui::test]
fn a_real_control_click_edits_the_document_and_undo_reverts(cx: &mut TestAppContext) {
    let rig = rig("timeline", 2);
    let first = publication(&rig);
    let (view, mut vcx) = window(cx, &rig, first, Default::default());
    vcx.run_until_parked();
    let id = view.read_with(&vcx, |v, _| v.semantic_id_starting("timeline:solo:"));
    let name = id.unwrap();
    let at = bounds(&mut vcx, &name).center();
    let solo = |rig: &Rig| {
        let p = rig.kernel.published(PROJECT).unwrap();
        pm_kernel::model::list_layers(p.snapshot.root().unwrap(), None)[0].solo
    };
    assert!(solo(&rig));
    vcx.simulate_click(at, Modifiers::none());
    let check = rig.kernel.clone();
    settle(cx, move |_| {
        !pm_kernel::model::list_layers(
            check.published(PROJECT).unwrap().snapshot.root().unwrap(),
            None,
        )[0]
        .solo
    });
    rig.kernel
        .call(
            PROJECT,
            pm_kernel::Request::new(
                pm_kernel::Caller::native(),
                pm_kernel::Call::History(pm_kernel::HistoryCall::Undo),
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(solo(&rig));
}
/// Dragging a real Properties number field writes through one kernel gesture.
#[gpui::test]
fn dragging_a_number_field_scrubs_as_one_undo_step(cx: &mut TestAppContext) {
    let rig = rig("inspector", 1);
    rig.kernel
        .call(
            PROJECT,
            pm_kernel::Request::new(
                pm_kernel::Caller::native(),
                pm_kernel::SelectionCall::Layers {
                    ids: vec!["l0".into()],
                    add: false,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    for (_, ticket) in rig.runtime.publish() {
        let ticket = ticket.unwrap();
        while ticket.try_take().is_none() {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let first = publication(&rig);
    let (view, mut vcx) = window(cx, &rig, first, Default::default());
    vcx.run_until_parked();
    let name = view
        .read_with(&vcx, |v, _| v.first_number())
        .expect("a bound number field");
    let at = bounds(&mut vcx, &name).center();
    let opacity = |rig: &Rig| rig.kernel.published(PROJECT).unwrap().revision();
    let start = opacity(&rig);
    let history = |rig: &Rig| rig.kernel.published(PROJECT).unwrap().history.can_undo;
    assert!(!history(&rig));
    vcx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    for step in 1..=5 {
        vcx.simulate_mouse_move(
            at + point(px(step as f32 * 8.), px(0.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        settle(cx, |_| false_after(10));
    }
    vcx.simulate_mouse_up(
        at + point(px(40.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    let check = rig.kernel.clone();
    settle(cx, move |_| {
        check.published(PROJECT).unwrap().history.can_undo
    });
    assert!(view.read_with(&vcx, |v, _| v.error.clone()).is_none());
    assert!(opacity(&rig) > start);
    let root = rig.kernel.published(PROJECT).unwrap();
    let edited = root.snapshot.root().unwrap().to_value();
    rig.kernel
        .call(
            PROJECT,
            pm_kernel::Request::new(
                pm_kernel::Caller::native(),
                pm_kernel::Call::History(pm_kernel::HistoryCall::Undo),
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let p = rig.kernel.published(PROJECT).unwrap();
    assert_ne!(p.snapshot.root().unwrap().to_value(), edited);
    assert!(!p.history.can_undo, "the whole scrub was one undo step");
}
fn false_after(ms: u64) -> bool {
    std::thread::sleep(Duration::from_millis(ms));
    true
}
/// S3: a real panel and its real V8 worker under the production gate.
#[gpui::test]
fn playback_never_reaches_a_panel_between_its_start_and_end(cx: &mut TestAppContext) {
    let rig = rig("timeline", 2);
    let first = publication(&rig);
    let (view, mut vcx) = window(cx, &rig, first, Default::default());
    vcx.run_until_parked();
    let baseline = view.read_with(&vcx, |v, _| v.renders);
    let mut clock = crate::playback::PanelClock::default();
    let mut deliveries = 0;
    let mut tick = |time: f64, playing: bool, vcx: &mut VisualTestContext| {
        if !clock.due(time, playing) {
            return;
        }
        deliveries += 1;
        rig.runtime
            .update_facts(json!({"time":time,"playing":playing}));
        for (_, ticket) in rig.runtime.publish() {
            let ticket = ticket.unwrap();
            while ticket.try_take().is_none() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        for p in rig.runtime.take_publications() {
            view.update(vcx, |v, cx| v.publish(p, cx));
        }
        vcx.run_until_parked();
    };
    tick(0., true, &mut vcx);
    let playing = view.read_with(&vcx, |v, _| v.renders);
    for frame in 1..600 {
        tick(frame as f64 / 120., true, &mut vcx);
    }
    assert_eq!(view.read_with(&vcx, |v, _| v.renders), playing);
    tick(5., false, &mut vcx);
    assert!(view.read_with(&vcx, |v, _| v.renders) > playing);
    tick(5.5, false, &mut vcx);
    assert_eq!(deliveries, 3);
    assert!(playing > baseline, "the play button changes once");
}
