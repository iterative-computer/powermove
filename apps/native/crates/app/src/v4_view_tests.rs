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
    rig_source(
        panel,
        OpenSource::Untitled(pm_model::json::parse(project).unwrap()),
    )
}
pub(crate) fn rig_source(panel: &str, source: OpenSource) -> Rig {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    kernel.open(PROJECT, source).wait().unwrap().unwrap();
    if let Ok(comp_id) = std::env::var("PM_PANEL_CAPTURE_COMP") {
        let document = kernel.published(PROJECT).unwrap().snapshot.doc_id;
        kernel
            .call(
                PROJECT,
                pm_kernel::Request::new(
                    pm_kernel::Caller::native(),
                    pm_kernel::Call::OpenComposition { comp_id, document },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
    }
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
/// A reloaded extension or a swapped fork reuses node ids under a new
/// generation: nothing bound to the old tree (here a pointer capture) carries over.
#[gpui::test]
fn a_new_generation_drops_what_the_old_tree_held(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let tree = || {
        let mut ops = node_ops(1, 0, "div", json!({"style":{"width":100,"height":40}}), &[]);
        ops.extend(node_ops(
            2,
            1,
            "div",
            json!({"id":"target","style":{"width":60,"height":20}}),
            &["pointerdown", "pointermove", "pointerup", "click"],
        ));
        ops
    };
    let (view, mut vcx) = window(cx, &rig, synthetic(&rig, tree()), Default::default());
    vcx.run_until_parked();
    let at = bounds(&mut vcx, "target").center();
    vcx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    let mut next = synthetic(&rig, tree());
    next.generation = 2;
    view.update(&mut vcx, |v, cx| v.publish(next, cx));
    vcx.run_until_parked();
    vcx.simulate_mouse_move(
        point(px(900.), px(500.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.run_until_parked();
    assert_eq!(view.read_with(&vcx, |v, _| v.publication.generation), 2);
    assert_eq!(events(&view, &mut vcx), vec![(2, "pointerdown".to_owned())]);
    // The new tree takes input at once.
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
    assert!(
        events(&view, &mut vcx).contains(&(2, "click".to_owned())),
        "a click after the swap reaches the new tree"
    );
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
    assert_eq!(names.len(), 34);
    for (i, name) in names.iter().enumerate() {
        let primitive = match name.as_str() {
            "sheet-anchor" | "popover-anchor" | "colour-popover" => "anchored",
            "number-field" | "text-field" | "slider" => "input",
            _ => "div",
        };
        let mut props = json!({"id":name,"data-pm-component":name,"label":name,"style":{"height":16,"width":20}});
        match name.as_str() {
            "number-field" => props["control"] = json!("number"),
            "text-field" => props["control"] = json!("text"),
            "slider" => {
                props["control"] = json!("slider");
                props["value"] = json!(0.5);
            }
            "progress-row" => props["aria-valuenow"] = json!(0.5),
            "colour-well" => props["value"] = json!("#ff0000"),
            "viewer-gizmo" => {
                props["metadata"] = json!({"pmKit":{"kind":"transform","position":{"x":4.,"y":5.}}})
            }
            "color-wheel" => {
                props["value"] = json!([0.1, 0., -0.1]);
                props["binding"] = json!("r+g+b");
                props["metadata"] = json!({"pmKit":{"neutral":0.,"min":-1.,"max":1.}});
            }
            "curve-editor" => {
                props["value"] = json!([2., 0., 0., 1., 1.]);
                props["binding"] = json!("curve");
                props["metadata"] = json!({"pmKit":{"channel":1}});
            }
            "ease-curve" | "ease-thumbnail" => {
                props["value"] = json!([0.42, 0., 0.58, 1.]);
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
/// A wheel step on a panel's scroller (PM-311) moves its rows without the
/// panel rendering again: the pointer lands on the row drawn under it, a step
/// past what was recorded renders, and the panel renders once the steps stop.
#[gpui::test]
fn a_wheel_step_moves_the_rows_without_rendering_the_panel(cx: &mut TestAppContext) {
    wheel_steps(cx, pm_builtins::PROJECT_PANEL, true);
}
/// Elsewhere a step draws the panel again (its hover and controls follow
/// the content at once: PM-389, PM-390).
#[gpui::test]
fn outside_the_project_panel_a_wheel_step_draws_the_panel_again(cx: &mut TestAppContext) {
    wheel_steps(cx, "fxbrowser", false);
}
fn wheel_steps(cx: &mut TestAppContext, panel: &str, layer: bool) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let mut ops = node_ops(
        1,
        0,
        "scroll",
        json!({"style":{"width":100,"height":100}}),
        &[],
    );
    for i in 0..20u64 {
        ops.extend(node_ops(
            2 + i,
            1,
            "div",
            json!({"id": format!("row-{i}"), "style":{"width":100,"height":20,"flexShrink":0}}),
            &["pointerdown"],
        ));
    }
    let mut surface = synthetic(&rig, ops);
    surface.panel = panel.into();
    let runtime = rig.runtime.clone();
    cx.executor().allow_parking();
    let (root, vcx) = cx.add_window_view(|window, cx| {
        let child = cx
            .new(|cx| SurfaceView::new(runtime, surface, tokens(), Default::default(), window, cx));
        Slot { child }
    });
    let mut vcx = vcx.clone();
    vcx.run_until_parked();
    let view = root.read_with(&vcx, |r, _| r.child.clone());
    let row = |vcx: &mut VisualTestContext, i: u64| bounds(vcx, &format!("row-{i}")).origin.y;
    let top = row(&mut vcx, 0);
    let at = point(px(50.), top + px(55.));
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.run_until_parked();
    let rendered = view.read_with(&vcx, |v, _| v.renders);
    let wheel = |vcx: &mut VisualTestContext, dy: f32| {
        vcx.simulate_event(ScrollWheelEvent {
            position: at,
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            ..Default::default()
        });
        vcx.run_until_parked();
    };
    wheel(&mut vcx, -30.);
    if !layer {
        assert!(view.read_with(&vcx, |v, _| v.renders) > rendered);
        assert_eq!(row(&mut vcx, 4), top + px(50.));
        return;
    }
    assert_eq!(view.read_with(&vcx, |v, _| v.renders), rendered);
    // 55 + 30 lands in row 4 (80..100) now.
    events(&view, &mut vcx);
    vcx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(events(&view, &mut vcx), vec![(6, "pointerdown".to_owned())]);
    // Another step, then the steps stop: the panel renders once, its rows
    // where they were shown.
    vcx.executor().advance_clock(std::time::Duration::from_millis(400));
    vcx.run_until_parked();
    let rendered = view.read_with(&vcx, |v, _| v.renders);
    wheel(&mut vcx, -20.);
    assert_eq!(view.read_with(&vcx, |v, _| v.renders), rendered);
    vcx.executor().advance_clock(std::time::Duration::from_millis(400));
    vcx.run_until_parked();
    assert_eq!(view.read_with(&vcx, |v, _| v.renders), rendered + 1);
    assert_eq!(row(&mut vcx, 4), top + px(30.));
    // A step past what was recorded (one height each way) renders at once.
    let rendered = view.read_with(&vcx, |v, _| v.renders);
    wheel(&mut vcx, -150.);
    assert_eq!(view.read_with(&vcx, |v, _| v.renders), rendered + 1);
    assert_eq!(row(&mut vcx, 10), top + px(200. - 200.));
}
/// A panel in a 100 pt slot, as a dock gives one.
/// PM-387: a list built while its slot was short asks for the rows that show
/// once the slot grows (its page held only the rows of the short slot).
#[gpui::test]
fn a_list_that_grows_taller_asks_for_the_rows_that_now_show(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let ops = node_ops(
        1,
        0,
        "virtual-list",
        json!({"count": 100, "range": [0, 2], "itemSize": 20, "style": {"width": 100}}),
        &["rangechange"],
    );
    let surface = synthetic(&rig, ops);
    let runtime = rig.runtime.clone();
    cx.executor().allow_parking();
    let (root, vcx) = cx.add_window_view(|window, cx| {
        let child = cx
            .new(|cx| SurfaceView::new(runtime, surface, tokens(), Default::default(), window, cx));
        Tall { child, height: 40. }
    });
    let mut vcx = vcx.clone();
    vcx.run_until_parked();
    let view = root.read_with(&vcx, |r, _| r.child.clone());
    let last_range = |vcx: &mut VisualTestContext| {
        view.update(vcx, |v, _| {
            v.log
                .iter()
                .rev()
                .find(|(_, e, _)| e == "rangechange")
                .and_then(|(_, _, d)| d.range)
        })
    };
    assert_eq!(last_range(&mut vcx), Some((0, 2)));
    root.update(&mut vcx, |r, cx| {
        r.height = 200.;
        cx.notify();
    });
    vcx.run_until_parked();
    assert_eq!(last_range(&mut vcx), Some((0, 10)));
}
/// A slot whose height the test sets.
struct Tall {
    child: Entity<SurfaceView>,
    height: f32,
}
impl Render for Tall {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(100.)).h(px(self.height)).flex().flex_col().child(
            self.child
                .clone()
                .cached(StyleRefinement::default().w(px(100.)).h(px(self.height))),
        )
    }
}
struct Slot {
    child: Entity<SurfaceView>,
}
impl Render for Slot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(100.)).child(
            self.child
                .clone()
                .cached(StyleRefinement::default().size(px(100.))),
        )
    }
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
    // A layer row's Visibility switch (always on the row).
    let id = view.read_with(&vcx, |v, _| v.semantic_id_starting("timeline:visibility:"));
    let name = id.unwrap();
    let at = bounds(&mut vcx, &name).center();
    // Every layer shows; the click hides the row's layer.
    let visible = |kernel: &pm_kernel::KernelHandle| {
        let p = kernel.published(PROJECT).unwrap();
        pm_kernel::model::list_layers(p.snapshot.root().unwrap(), None)
            .iter()
            .all(|l| l.visible)
    };
    assert!(visible(&rig.kernel));
    vcx.simulate_click(at, Modifiers::none());
    let check = rig.kernel.clone();
    settle(cx, move |_| !visible(&check));
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
    assert!(visible(&rig.kernel));
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
/// Headless input: panel handoff, phase filtering and one kernel undo.
#[gpui::test]
fn horizontal_wheel_scrub_is_one_undo_and_ignores_momentum(cx: &mut TestAppContext) {
    wheel_scrub_scenario(cx, false);
}
#[gpui::test]
fn horizontal_wheel_scrub_adds_an_animated_key(cx: &mut TestAppContext) {
    wheel_scrub_scenario(cx, true);
}
fn wheel_scrub_scenario(cx: &mut TestAppContext, animated: bool) {
    let mut project: Value = serde_json::from_str(&project_json(1)).unwrap();
    if animated {
        project["layers"][0]["p"]["opacity"]["kf"] = json!([{"i":"existing","t":1,"v":100}]);
    }
    let rig = rig_project("inspector", &project.to_string());
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
        .read_with(&vcx, |v, _| v.number_containing("opacity"))
        .unwrap();
    let n = view.read_with(&vcx, |v, _| v.numeric_facts(&name));
    let at = bounds(&mut vcx, &name).center();
    let before = rig
        .kernel
        .published(PROJECT)
        .unwrap()
        .snapshot
        .root()
        .unwrap()
        .to_value();
    let wheel = |vcx: &mut VisualTestContext, x: f32, y: f32, phase| {
        vcx.simulate_event(ScrollWheelEvent {
            position: at,
            delta: ScrollDelta::Pixels(point(px(x), px(y))),
            touch_phase: phase,
            ..Default::default()
        });
    };
    wheel(&mut vcx, 0., 24., gpui::TouchPhase::Started);
    assert!(!rig.kernel.published(PROJECT).unwrap().history.can_undo);
    wheel(&mut vcx, 24., 0., gpui::TouchPhase::Started);
    wheel(&mut vcx, 24., 0., gpui::TouchPhase::Moved);
    wheel(&mut vcx, 0., 0., gpui::TouchPhase::Ended);
    wheel(&mut vcx, 500., 0., gpui::TouchPhase::Moved);
    let expected = (n.value - 48. * 0.25 * n.step * n.speed).clamp(n.min, n.max);
    assert_eq!(
        view.read_with(&vcx, |v, _| v.wheel_reading()),
        Some((expected * 10_000.).round() / 10_000.)
    );
    vcx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(301));
    let check = rig.kernel.clone();
    settle(cx, move |_| {
        check.published(PROJECT).unwrap().history.can_undo
    });
    assert!(view.read_with(&vcx, |v, _| v.error.clone()).is_none());
    if animated {
        let p = rig.kernel.published(PROJECT).unwrap();
        let root = p.snapshot.root().unwrap();
        let at = pm_kernel::model::find_layer(root, "l0").unwrap();
        let layer = pm_kernel::model::layer_at(root, &at).unwrap().to_value();
        let keys = layer
            .get("p")
            .and_then(|p| p.get("opacity"))
            .and_then(|p| p.get("kf"))
            .and_then(pm_model::Value::as_array)
            .unwrap();
        assert_eq!(keys.len(), 2, "scrubbing adds a key at the playhead");
        assert!(
            keys.iter()
                .any(|k| k.get("t").and_then(pm_model::Value::as_f64) == Some(0.)
                    && k.get("v").and_then(pm_model::Value::as_f64) == Some(expected))
        );
    }
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
    let after = rig.kernel.published(PROJECT).unwrap();
    assert_eq!(after.snapshot.root().unwrap().to_value(), before);
    assert!(!after.history.can_undo);
}
/// Timeline's expanded rows mount exactly the same field, with a real lease.
#[gpui::test]
fn timeline_value_drag_is_one_undo_and_click_opens_text(cx: &mut TestAppContext) {
    let mut project: serde_json::Value = serde_json::from_str(&project_json(1)).unwrap();
    project["layers"][0]["p"]["opacity"]["kf"] =
        serde_json::json!([{"i":"opacity0","t":0,"v":100}]);
    let rig = rig_project("timeline", &project.to_string());
    let first = publication(&rig);
    let (view, mut vcx) = window(cx, &rig, first, Default::default());
    vcx.run_until_parked();
    let disclosure = view
        .read_with(&vcx, |v, _| v.semantic_id_starting("timeline:disclosure:"))
        .unwrap();
    view.update(&mut vcx, |v, cx| v.click_semantic(&disclosure, cx));
    settle(cx, |_| {
        let mut context = vcx.clone();
        for p in rig.runtime.take_publications() {
            view.update(&mut context, |v, cx| v.publish(p, cx));
        }
        view.read_with(&context, |v, _| v.first_number().is_some())
    });
    vcx.run_until_parked();
    let name = view
        .read_with(&vcx, |v, _| v.first_number())
        .expect("timeline uses shared numeric field");
    let at = bounds(&mut vcx, &name).center();
    let before = rig
        .kernel
        .published(PROJECT)
        .unwrap()
        .snapshot
        .root()
        .unwrap()
        .to_value();
    vcx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    for step in 1..=4 {
        vcx.simulate_mouse_move(
            at + point(px(-8. * step as f32), px(0.)),
            MouseButton::Left,
            Modifiers::none(),
        );
    }
    vcx.simulate_mouse_up(
        at + point(px(-32.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    let check = rig.kernel.clone();
    settle(cx, move |_| {
        check.published(PROJECT).unwrap().history.can_undo
    });
    assert_ne!(
        rig.kernel
            .published(PROJECT)
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .to_value(),
        before
    );
    assert!(view.read_with(&vcx, |v, _| v.error.clone()).is_none());
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
    assert_eq!(p.snapshot.root().unwrap().to_value(), before);
    assert!(!p.history.can_undo);
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
    assert!(
        view.read_with(&vcx, |v, _| v.number_edit_requested(&name)),
        "a click opens the native text editor"
    );
}
/// A scrub whose release never reaches the window (AppKit delivered it
/// elsewhere, owner feedback W4/W5) ends at the next move with the button
/// up, as one Undo step, and stops holding the window's pointer moves.
#[gpui::test]
fn a_scrub_whose_release_is_lost_ends_on_the_next_move(cx: &mut TestAppContext) {
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
    vcx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    for step in 1..=5 {
        vcx.simulate_mouse_move(
            at + point(px(step as f32 * 8.), px(0.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        settle(cx, |_| false_after(10));
    }
    assert!(view.read_with(&vcx, |v, _| v.scrubbing()));
    // No release; the pointer moves on with the button up.
    vcx.simulate_mouse_move(at + point(px(60.), px(20.)), None, Modifiers::none());
    assert!(
        !view.read_with(&vcx, |v, _| v.scrubbing()),
        "the scrub ended"
    );
    let check = rig.kernel.clone();
    settle(cx, move |_| {
        check.published(PROJECT).unwrap().history.can_undo
    });
    assert!(rig.kernel.published(PROJECT).unwrap().history.can_undo);
    // Again, and this time the next thing is a press elsewhere: the stale
    // scrub ends before that press is handled.
    vcx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    for step in 1..=3 {
        vcx.simulate_mouse_move(
            at + point(px(step as f32 * 8.), px(0.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        settle(cx, |_| false_after(10));
    }
    assert!(view.read_with(&vcx, |v, _| v.scrubbing()));
    vcx.simulate_mouse_down(
        at + point(px(0.), px(400.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    assert!(
        !view.read_with(&vcx, |v, _| v.scrubbing()),
        "the next press ended the scrub"
    );
}
fn false_after(ms: u64) -> bool {
    std::thread::sleep(Duration::from_millis(ms));
    true
}
/// Project rows and effect rows open their menus from this press.
#[gpui::test]
fn a_secondary_press_reaches_pointerdown_with_its_button(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let mut ops = node_ops(1, 0, "div", json!({"style":{"width":100,"height":40}}), &[]);
    ops.extend(node_ops(
        2,
        1,
        "div",
        json!({"id":"target","style":{"width":60,"height":20}}),
        &["pointerdown"],
    ));
    let (view, mut vcx) = window(cx, &rig, synthetic(&rig, ops), Default::default());
    vcx.run_until_parked();
    let at = bounds(&mut vcx, "target").center();
    vcx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
    let log = view.update(&mut vcx, |v, _| std::mem::take(&mut v.log));
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].1, "pointerdown");
    assert_eq!(log[0].2.button, Some(2));
}
/// E3: a sheet that just opened has nothing focused; its window hands Return
/// and Escape to the panel's root, where the sheet presses its default
/// button or cancels.
#[gpui::test]
fn a_key_the_window_hands_over_reaches_the_panels_root(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let ops = node_ops(
        1,
        0,
        "div",
        json!({"id":"sheet","style":{"width":100,"height":40}}),
        &["keydown"],
    );
    let (view, mut vcx) = window(cx, &rig, synthetic(&rig, ops), Default::default());
    vcx.run_until_parked();
    for key in ["enter", "escape"] {
        let event = gpui::KeyDownEvent {
            keystroke: gpui::Keystroke::parse(key).unwrap(),
            is_held: false,
            prefer_character_input: false,
        };
        vcx.update(|window, cx| view.update(cx, |v, cx| v.root_key_down(&event, window, cx)));
    }
    vcx.run_until_parked();
    let log = view.update(&mut vcx, |v, _| std::mem::take(&mut v.log));
    let keys: Vec<_> = log
        .iter()
        .map(|(id, e, d)| (*id, e.clone(), d.key.clone()))
        .collect();
    assert_eq!(
        keys,
        [
            (1, "keydown".to_owned(), Some("Enter".to_owned())),
            (1, "keydown".to_owned(), Some("Escape".to_owned())),
        ]
    );
}
/// U4: a scroller that asks to keep rows keeps them in a body too short for
/// its panel (the body scrolls instead); one that does not may shrink away.
#[gpui::test]
fn a_scroller_keeps_the_rows_its_panel_asks_for_in_a_short_body(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let column = json!({"display":"flex","flexDirection":"column","width":200,"height":60});
    let mut ops = node_ops(1, 0, "div", json!({ "style": column }), &[]);
    ops.extend(node_ops(
        2,
        1,
        "div",
        json!({"style":{"height":50,"flexShrink":0}}),
        &[],
    ));
    ops.extend(node_ops(
        3,
        1,
        "scroll",
        json!({"metadata":{"rowFloor":3}}),
        &[],
    ));
    ops.extend(node_ops(4, 3, "div", json!({"style":{"height":500}}), &[]));
    ops.extend(node_ops(5, 1, "scroll", json!({}), &[]));
    ops.extend(node_ops(6, 5, "div", json!({"style":{"height":500}}), &[]));
    let (_view, mut vcx) = window(cx, &rig, synthetic(&rig, ops), Default::default());
    vcx.run_until_parked();
    let row = tokens().metrics.row;
    assert_eq!(f32::from(bounds(&mut vcx, "n3").size.height), 3. * row);
    assert!(f32::from(bounds(&mut vcx, "n5").size.height) < row);
}
/// A side panel's body as `panel_body` builds it: a slot that scrolls, the
/// panel as tall as its content and at least the slot.
struct ScrollingBody {
    view: Entity<SurfaceView>,
    scroll: gpui::ScrollHandle,
}
impl Render for ScrollingBody {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui::{InteractiveElement, StatefulInteractiveElement};
        div().w(px(200.)).h(px(60.)).child(
            div()
                .id("body")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child({
                    let content = self.view.read(cx).in_body(60.);
                    self.view.clone().cached(content)
                }),
        )
    }
}
/// U4: a side panel taller than its body grows with its rows (the body then
/// scrolls to them); a panel that fits still fills its body.
#[gpui::test]
fn a_side_panel_taller_than_its_body_gives_the_body_something_to_scroll(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    for (rows, scrolls) in [(3, true), (0, false)] {
        let column = json!({"display":"flex","flexDirection":"column","flexGrow":1});
        let mut ops = node_ops(1, 0, "div", json!({ "style": column }), &[]);
        ops.extend(node_ops(
            2,
            1,
            "div",
            json!({"style":{"height":50,"flexShrink":0}}),
            &[],
        ));
        ops.extend(node_ops(
            3,
            1,
            "scroll",
            json!({"metadata":{"rowFloor":rows}}),
            &[],
        ));
        ops.extend(node_ops(4, 3, "div", json!({"style":{"height":500}}), &[]));
        cx.executor().allow_parking();
        let runtime = rig.runtime.clone();
        let first = synthetic(&rig, ops);
        let scroll = gpui::ScrollHandle::new();
        let handle = scroll.clone();
        let (_, mut vcx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut v =
                    SurfaceView::new(runtime, first, tokens(), Default::default(), window, cx);
                v.grows = true;
                v
            });
            ScrollingBody {
                view,
                scroll: handle,
            }
        });
        vcx.simulate_resize(size(px(400.), px(300.)));
        vcx.run_until_parked();
        // The first frame measures the panel; the next sizes the body by it.
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let reach = f32::from(scroll.max_offset().y);
        eprintln!(
            "DEBUG rows={rows} reach={reach} n1={:?} n3={:?}",
            bounds(&mut vcx, "n1"),
            bounds(&mut vcx, "n3")
        );
        if scrolls {
            let row = tokens().metrics.row;
            assert_eq!(reach, 50. + 3. * row - 60., "the rows are within reach");
        } else {
            assert_eq!(reach, 0., "a panel that fits does not scroll");
            // Its list fills what the header leaves.
            assert_eq!(f32::from(bounds(&mut vcx, "n3").size.height), 10.);
        }
    }
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

#[::core::prelude::v1::test]
fn status_nodes_announce_when_they_appear_or_change_never_on_open() {
    use crate::v4_view::status_announcements;
    let mut seen = None;
    let status = |id: u64, text: &str| (id, text.to_owned());
    // Opening state is recorded, not read out.
    assert!(status_announcements(&mut seen, [status(4, "Layer is locked")]).is_empty());
    // Unchanged: silent. Changed or new: announced. Empty: silent.
    assert!(status_announcements(&mut seen, [status(4, "Layer is locked")]).is_empty());
    assert_eq!(
        status_announcements(
            &mut seen,
            [status(4, "Removed marker"), status(9, "2 layers selected")]
        ),
        ["Removed marker", "2 layers selected"]
    );
    assert!(status_announcements(&mut seen, [status(4, "")]).is_empty());
    assert_eq!(
        status_announcements(&mut seen, [status(4, "3 layers selected")]),
        ["3 layers selected"]
    );
}
/// v1's `inspector.focusText(layer)`: asked while another text layer is
/// still selected, Properties waits for the text row bound to that layer
/// (never the other's), and gives up after a moment. The AppKit field's
/// focus and selection need a real window (checked on screen).
#[gpui::test]
fn focus_text_waits_for_the_text_row_of_its_own_layer(cx: &mut TestAppContext) {
    let text = |id: &str, text: &str| {
        format!(
            r##"{{"id":"{id}","type":"text","name":"{id}","from":0,"dur":2,"on":true,"p":{{"opacity":{{"v":100,"kf":[]}},"x":{{"v":10,"kf":[]}},"y":{{"v":20,"kf":[]}},"sx":{{"v":100,"kf":[]}},"sy":{{"v":100,"kf":[]}},"r":{{"v":0,"kf":[]}}}},"d":{{"text":"{text}"}},"fx":[]}},"##
        )
    };
    let project = project_json(1).replace(
        r#""layers":["#,
        &format!(
            r#""layers":[{}{}"#,
            text("t1", "Hello"),
            text("t2", "Other")
        ),
    );
    let rig = rig_project("inspector", &project);
    let select = |id: &str| {
        rig.kernel
            .call(
                PROJECT,
                pm_kernel::Request::new(
                    pm_kernel::Caller::native(),
                    pm_kernel::SelectionCall::Layers {
                        ids: vec![id.into()],
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
    };
    select("t2");
    let first = publication(&rig);
    let (view, mut vcx) = window(cx, &rig, first, Default::default());
    vcx.run_until_parked();
    let row = |view: &Entity<SurfaceView>, vcx: &mut VisualTestContext| {
        view.read_with(vcx, |v, _| v.focus_text_row())
    };
    view.update(&mut vcx, |v, cx| v.focus_text("t1".into(), cx));
    assert_eq!(row(&view, &mut vcx), None, "t2's text row is not t1's");
    select("t1");
    let publish = |view: &Entity<SurfaceView>, vcx: &mut VisualTestContext| {
        for p in rig.runtime.take_publications() {
            view.update(vcx, |v, cx| v.publish(p, cx));
        }
    };
    publish(&view, &mut vcx);
    let check = view.clone();
    settle(cx, move |cx| {
        check.read_with(cx, |v, _| v.focus_text_row().is_some())
    });
    // Asked and never shown: forgotten after a moment, not applied later.
    select("l0");
    publish(&view, &mut vcx);
    view.update(&mut vcx, |v, cx| v.focus_text("t2".into(), cx));
    std::thread::sleep(Duration::from_millis(2100));
    select("t2");
    publish(&view, &mut vcx);
    vcx.run_until_parked();
    settle(cx, |_| false_after(50));
    assert_eq!(row(&view, &mut vcx), None);
}

/// A panel window that hosts a kept view, as the app's sheet windows do: it
/// attaches the view on every render (`Controller::builtin_panel`).
struct SheetWindow(Entity<SurfaceView>);
impl Render for SheetWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.0.update(cx, |v, cx| v.attach(window, cx));
        div().size_full().child(self.0.clone())
    }
}
#[gpui::test]
fn a_reopened_sheet_shows_what_was_published_while_it_was_closed(cx: &mut TestAppContext) {
    let wait = |ticket: pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while ticket.try_take().is_none() {
            assert!(Instant::now() < deadline, "export realm did not settle");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let rig = rig("export", 1);
    wait(rig.runtime.show("export").unwrap());
    let (view, mut first) = window(cx, &rig, publication(&rig), Default::default());
    let texts = |cx: &mut TestAppContext| {
        view.read_with(cx, |v, _| {
            v.publication
                .surface
                .nodes()
                .values()
                .map(|n| n.text.clone())
                .collect::<Vec<_>>()
        })
    };
    let deliver = |cx: &mut TestAppContext| {
        for p in rig.runtime.take_publications() {
            view.update(cx, |v, cx| v.publish(p, cx));
        }
    };
    // The export runs to its last frame while the sheet is open.
    wait(
        rig.runtime
            .observe_export(json!({"phase":"rendering","frame":300,"total":300,"elapsed":2}))
            .unwrap(),
    );
    deliver(cx);
    settle(cx, |cx| {
        texts(cx).iter().any(|t| t.starts_with("Exporting"))
    });
    // It finishes: the app closes the sheet's window, then the realm
    // publishes the finished run; the person opens Export again and the
    // realm publishes a fresh sheet before the new window exists.
    first.update(|window, _| window.remove_window());
    cx.run_until_parked();
    wait(
        rig.runtime
            .observe_export(json!({"phase":"done","path":"/tmp/Synthetic Film.mp4"}))
            .unwrap(),
    );
    deliver(cx);
    wait(rig.runtime.show("export").unwrap());
    deliver(cx);
    cx.run_until_parked();
    let kept = view.clone();
    cx.add_window(|_, _| SheetWindow(kept));
    settle(cx, |cx| {
        let texts = texts(cx);
        !texts.iter().any(|t| t.starts_with("Exporting")) && texts.iter().any(|t| t == "Export")
    });
}

/// Cancel reaches the app however the export began: Export Video, the
/// refusal naming offline clips, Export Without Them, then Cancel while it
/// renders sends `export.cancel` (the sheet closed and the export ran on).
#[gpui::test]
fn cancel_reaches_the_app_after_export_without_them(cx: &mut TestAppContext) {
    let wait = |ticket: pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while ticket.try_take().is_none() {
            assert!(Instant::now() < deadline, "export realm did not settle");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let rig = rig_project(
        "export",
        r##"{"id":"integration","name":"Strips","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","exportDefaults":{"format":"mp4"},"assets":{"a0":{"id":"a0","kind":"video","dur":10}},"comps":{},"layers":[]}"##,
    );
    wait(rig.runtime.show("export").unwrap());
    let (view, mut vcx) = window(cx, &rig, publication(&rig), Default::default());
    let mut sent: Vec<String> = Vec::new();
    // Publications to the sheet, the host's platform requests answered as
    // the app answers them, until `operation` is asked for.
    let mut until_asked = |vcx: &mut VisualTestContext, operation: &str| {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            for p in rig.runtime.take_publications() {
                view.update(vcx, |v, cx| v.publish(p, cx));
            }
            vcx.run_until_parked();
            for r in rig.runtime.take_platform_requests() {
                sent.push(r.operation.clone());
                rig.runtime.platform_reply(r.id, Ok(Value::Null));
            }
            if sent.iter().any(|s| s == operation) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{operation} never asked; asked {sent:?}; errors {:?}",
                rig.runtime.take_errors()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    // Clicks a control once it is painted (publications keep arriving).
    let click = |vcx: &mut VisualTestContext, id: &str| {
        let deadline = Instant::now() + Duration::from_secs(20);
        let b = loop {
            // As the app does on a wake: the sheet's finished encoder checks
            // are published (Export Video is off until they are).
            if let Some(Ok(ticket)) = rig.runtime.publish_finished_checks() {
                wait(ticket);
            }
            for p in rig.runtime.take_publications() {
                view.update(vcx, |v, cx| v.publish(p, cx));
            }
            vcx.run_until_parked();
            if let Some(b) = vcx.debug_bounds(Box::leak(id.to_owned().into_boxed_str())) {
                break b;
            }
            assert!(Instant::now() < deadline, "{id} never painted");
            std::thread::sleep(Duration::from_millis(2));
        };
        vcx.simulate_click(b.center(), Modifiers::none());
        vcx.run_until_parked();
    };
    // Export Video is on once the sheet's encoder checks have finished and
    // been published.
    let checked = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(Ok(ticket)) = rig.runtime.publish_finished_checks() {
            wait(ticket);
            break;
        }
        assert!(Instant::now() < checked, "the encoder checks never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    click(&mut vcx, "export.start");
    until_asked(&mut vcx, "export.begin");
    wait(
        rig.runtime
            .observe_export(json!({"phase":"failed","message":"1 clip is offline: a.mov. Relink the originals, or export without them.","offline":true}))
            .unwrap(),
    );
    until_asked(&mut vcx, "export.begin");
    click(&mut vcx, "export.without");
    until_asked(&mut vcx, "export.without");
    wait(
        rig.runtime
            .observe_export(json!({"phase":"rendering","frame":12,"total":300,"elapsed":2}))
            .unwrap(),
    );
    until_asked(&mut vcx, "export.without");
    click(&mut vcx, "export.cancel");
    until_asked(&mut vcx, "export.cancel");
}

#[gpui::test]
fn a_disabled_field_still_shows_its_value(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let mut ops = node_ops(
        1,
        0,
        "div",
        json!({"style":{"width":400,"height":200}}),
        &[],
    );
    ops.extend(node_ops(
        2,
        1,
        "input",
        json!({"id":"source","data-pm-component":"text-field","control":"text","label":"Source","value":"Cube","disabled":true}),
        &[],
    ));
    ops.extend(node_ops(
        3,
        1,
        "input",
        json!({"id":"depth","data-pm-component":"number-field","control":"number","label":"Depth","value":2.5,"unit":"px","disabled":true}),
        &[],
    ));
    ops.extend(node_ops(
        4,
        1,
        "input",
        json!({"id":"live","data-pm-component":"text-field","control":"text","label":"Live","value":"Editable"}),
        &[],
    ));
    // A locked colour: its well and its hex in one value slot.
    ops.extend(node_ops(5, 1, "div", json!({"data-pm-slot":"value"}), &[]));
    ops.extend(node_ops(
        6,
        5,
        "button",
        json!({"id":"fill","data-pm-component":"colour-well","control":"colour","label":"Fill","value":"#6A6A70","disabled":true}),
        &[],
    ));
    ops.extend(node_ops(
        7,
        5,
        "input",
        json!({"id":"hex","data-pm-component":"text-field","control":"text","label":"Fill Hex","value":"#6A6A70","disabled":true,"style":{"background":{"token":"colors.field"}}}),
        &[],
    ));
    let (view, vcx) = window(cx, &rig, synthetic(&rig, ops), Default::default());
    vcx.run_until_parked();
    let hex = view.read_with(&vcx, |v, _| {
        v.prepared.nodes.get(&7).map(|n| {
            (
                n.bare,
                n.style.background.is_some(),
                v.disabled_field_text(n),
            )
        })
    });
    assert_eq!(
        hex,
        Some((true, false, Some("#6A6A70".into()))),
        "a locked hex is its value in the colour's well, with no well of its own"
    );
    let shown = view.read_with(&vcx, |v, _| {
        [2, 3, 4].map(|id| {
            v.prepared
                .nodes
                .get(&id)
                .and_then(|n| v.disabled_field_text(n))
        })
    });
    assert_eq!(shown[0].as_deref(), Some("Cube"));
    assert_eq!(shown[1].as_deref(), Some("2.5 px"));
    // An enabled field draws through its editor instead.
    assert_eq!(shown[2], None);
    // Once: the number decoration leaves a disabled field's value to it (a
    // locked layer's wells read "100 px100 px").
    let decorated = view.read_with(&vcx, |v, _| {
        v.prepared
            .nodes
            .get(&3)
            .map(|n| (n.number.is_some(), v.draws_number(n)))
    });
    assert_eq!(decorated, Some((true, false)));
}

/// The export sheet's footer laid out with `offline` unreadable originals in
/// a project export: the sheet, its notice and its two buttons.
fn export_footer(cx: &mut TestAppContext, offline: usize) -> [Bounds<Pixels>; 4] {
    let wait = |ticket: pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while ticket.try_take().is_none() {
            assert!(Instant::now() < deadline, "export realm did not settle");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let assets = (0..offline)
        .map(|i| format!(r#""a{i}":{{"id":"a{i}","kind":"video","dur":10}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let rig = rig_project(
        "export",
        &format!(
            r##"{{"id":"integration","name":"Strips","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","exportDefaults":{{"format":"json"}},"assets":{{{assets}}},"comps":{{}},"layers":[]}}"##
        ),
    );
    wait(rig.runtime.show("export").unwrap());
    let (view, mut vcx) = window(cx, &rig, publication(&rig), Default::default());
    let expected = match offline {
        0 => "Strips".to_owned(),
        n => format!("{n} media files are offline"),
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        for p in rig.runtime.take_publications() {
            view.update(&mut vcx, |v, cx| v.publish(p, cx));
        }
        vcx.run_until_parked();
        let shown = view.read_with(&vcx, |v, _| {
            v.publication
                .surface
                .nodes()
                .values()
                .any(|n| n.text.contains(&expected))
        });
        if shown {
            break;
        }
        assert!(Instant::now() < deadline, "{expected:?} never showed");
        std::thread::sleep(Duration::from_millis(1));
    }
    [
        "export.sheet",
        "export.summary",
        "export.cancel",
        "export.start",
    ]
    .map(|n| bounds(&mut vcx, n))
}

/// The footer keeps its buttons whole and inside the sheet however long its
/// notice runs: the notice wraps beside them (Cancel was squeezed, and the
/// AppKit button it hosts then pushed Export Video past the sheet's edge).
#[gpui::test]
fn a_long_export_notice_wraps_and_never_squeezes_the_buttons(cx: &mut TestAppContext) {
    let [_, _, cancel_alone, start_alone] = export_footer(cx, 0);
    let [sheet, summary, cancel, start] = export_footer(cx, 12);
    assert_eq!(
        (cancel.size.width, start.size.width),
        (cancel_alone.size.width, start_alone.size.width),
        "buttons squeezed by the notice"
    );
    assert!(
        start.right() <= sheet.right() && summary.right() <= cancel.left(),
        "notice {summary:?}, buttons {cancel:?} {start:?}, sheet {sheet:?}"
    );
    assert!(summary.size.height > cancel.size.height, "the notice wraps");
}

/// Full Keyboard Access: Tab reaches kit controls in order; `tabIndex` -1
/// (a segmented control's other segments) stays out of the Tab order.
#[gpui::test]
fn tab_reaches_kit_controls_and_skips_roving_segments(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let mut ops = node_ops(1, 0, "div", json!({"style":{"width":200,"height":40}}), &[]);
    for (i, (name, tab)) in [("first", 0), ("roving", -1), ("last", 0)].iter().enumerate() {
        ops.extend(node_ops(
            2 + i as u64,
            1,
            "button",
            json!({"id":name,"data-pm-component":"toolbar-item","label":name,"tabIndex":tab,"style":{"width":20,"height":16}}),
            &["click"],
        ));
    }
    let (view, mut vcx) = window(cx, &rig, synthetic(&rig, ops), Default::default());
    vcx.run_until_parked();
    let mut order = vec![];
    for _ in 0..2 {
        vcx.update(|window, cx| window.focus_next(cx));
        vcx.run_until_parked();
        let focused = view.update_in(&mut vcx, |v, window, _| v.focused_node(window));
        order.push(focused);
    }
    assert_eq!(order, [Some(2), Some(4)], "Tab skips the -1 segment");
}

/// A panel in a tab group: its tab names it, so its header leaves out its
/// own title (the duplicate "Project" beside the tabs) and keeps its actions.
#[gpui::test]
fn a_tabbed_panel_header_leaves_out_its_title(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let mut ops = node_ops(
        1,
        0,
        "div",
        json!({"style":{"width":400,"height":200}}),
        &[],
    );
    ops.extend(node_ops(2, 1, "div", json!({"data-pm-slot":"header"}), &[]));
    // As a panel writes it: a styled text holding its words.
    ops.extend(node_ops(
        3,
        2,
        "text",
        json!({"style":{"fontSize":{"token":"typography.title"}}}),
        &[],
    ));
    ops.extend(node_ops(4, 3, "text", json!({}), &[]));
    ops.push(Op::SetText {
        id: 4,
        text: "Project".into(),
    });
    ops.extend(node_ops(
        5,
        2,
        "button",
        json!({"id":"search","data-pm-component":"toolbar-item","label":"Search Media"}),
        &["click"],
    ));
    let (view, vcx) = window(cx, &rig, synthetic(&rig, ops), Default::default());
    vcx.run_until_parked();
    let (header, title) = view.read_with(&vcx, |v, _| {
        let header = v.prepared.header;
        (header, header.and_then(|h| v.header_title(h)))
    });
    assert_eq!(header, Some(2));
    assert_eq!(title, Some(3), "the title is its first text, not a control");
}

/// A tooltip says what is not in view: a row titled with the name it
/// already shows has none; an icon-only control keeps its title.
#[gpui::test]
fn a_tooltip_never_repeats_the_text_in_view(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let mut ops = node_ops(
        1,
        0,
        "div",
        json!({"style":{"width":400,"height":200}}),
        &[],
    );
    ops.extend(node_ops(
        2,
        1,
        "div",
        json!({"title":"Gaussian Blur","data-pm-component":"list-item","label":"Gaussian Blur"}),
        &["click"],
    ));
    ops.extend(node_ops(3, 2, "text", json!({}), &[]));
    ops.push(Op::SetText {
        id: 3,
        text: "Gaussian Blur".into(),
    });
    ops.extend(node_ops(
        4,
        1,
        "button",
        json!({"title":"Reset Effect","data-pm-component":"toolbar-item","label":"Reset Gaussian Blur","metadata":{"symbol":"arrow.counterclockwise"}}),
        &["click"],
    ));
    // A row's value well that opens a menu: its row says "Font", it shows
    // its value; an icon-only menu button beside it.
    ops.extend(node_ops(
        5,
        1,
        "div",
        json!({"data-pm-component":"inspector-row","label":"Font"}),
        &[],
    ));
    ops.extend(node_ops(
        6,
        5,
        "button",
        json!({"title":"Font","data-pm-component":"toolbar-item","label":"Font","aria-haspopup":"menu"}),
        &["click"],
    ));
    ops.extend(node_ops(7, 6, "text", json!({}), &[]));
    ops.push(Op::SetText {
        id: 7,
        text: "SF Pro Display".into(),
    });
    ops.extend(node_ops(
        8,
        1,
        "button",
        json!({"title":"More","data-pm-component":"toolbar-item","label":"More","aria-haspopup":"menu","metadata":{"symbol":"ellipsis"}}),
        &["click"],
    ));
    let (view, vcx) = window(cx, &rig, synthetic(&rig, ops), Default::default());
    vcx.run_until_parked();
    let shown = view.read_with(&vcx, |v, _| (v.shown_text(2, 3), v.shown_text(4, 3)));
    assert_eq!(shown.0, "Gaussian Blur", "the title is in view: no tooltip");
    assert_eq!(shown.1, "", "an icon says nothing: its title stays");
    let tips = view.read_with(&vcx, |v, _| {
        [2, 4, 6, 8].map(|id| v.tooltip_title(id).map(|t| t.to_string()))
    });
    assert_eq!(
        tips,
        [None, Some("Reset Effect".into()), None, Some("More".into())],
        "no tooltip repeats the view or stands over a value's menu; icons keep theirs"
    );
}
/// A tooltip shown before a press that changes its words (a disclosure's
/// Expand becomes Collapse when the panel republishes) says the new words
/// while the pointer stays.
#[gpui::test]
fn a_shown_tooltip_follows_its_nodes_title(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let tree = |title: &str| {
        let mut ops = node_ops(1, 0, "div", json!({"style":{"width":200,"height":200}}), &[]);
        ops.extend(node_ops(
            2,
            1,
            "button",
            json!({"title":title,"data-pm-component":"toolbar-item","label":title,
                "metadata":{"symbol":"chevron.right"}}),
            &["click"],
        ));
        ops
    };
    let (view, mut vcx) = window(cx, &rig, synthetic(&rig, tree("Expand Layer")), Default::default());
    vcx.run_until_parked();
    let tokens = view.read_with(&vcx, |v, _| v.tokens().clone());
    let tip = vcx.update(|_, cx| {
        cx.new(|_| {
            crate::v4_view::Tooltip::following(view.downgrade(), 2, "Expand Layer".into(), tokens)
        })
    });
    view.update(&mut vcx, |v, cx| v.publish(synthetic(&rig, tree("Collapse Layer")), cx));
    settle(cx, |_| true);
    vcx.run_until_parked();
    let shown = vcx.update(|_, cx| tip.update(cx, |t, cx| t.current_title(cx)));
    assert_eq!(shown.as_ref(), "Collapse Layer");
}
/// A section heading pressed closes its group in the next frame, before the
/// panel's script has answered; the publication that agrees takes over, and
/// expanding shows the open chevron at once while the content follows.
#[gpui::test]
fn a_disclosure_press_shows_in_the_next_frame(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let tree = |open: bool| {
        let mut ops = node_ops(
            1,
            0,
            "div",
            json!({"style":{"width":200,"height":200}}),
            &[],
        );
        ops.extend(node_ops(
            2,
            1,
            "div",
            json!({"data-pm-component":"disclosure-group","role":"group"}),
            &[],
        ));
        ops.extend(node_ops(
            3,
            2,
            "button",
            json!({"id":"heading","data-pm-component":"disclosure-button","aria-expanded":open,
                "label":"Transform","style":{"width":120,"height":20}}),
            &["click"],
        ));
        if open {
            ops.extend(node_ops(
                4,
                2,
                "div",
                json!({"id":"content","style":{"width":120,"height":60}}),
                &[],
            ));
        }
        ops
    };
    let (view, mut vcx) = window(cx, &rig, synthetic(&rig, tree(true)), Default::default());
    vcx.run_until_parked();
    let at = bounds(&mut vcx, "heading").center();
    assert!(vcx.debug_bounds("content").is_some());
    vcx.simulate_click(at, Modifiers::none());
    // The next frame: closed, with no publication in between.
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert!(!view.read_with(&vcx, |v, _| v.disclosure_open(3)));
    assert!(
        vcx.debug_bounds("content").is_none(),
        "the group closed at once"
    );
    assert!(
        events(&view, &mut vcx).contains(&(3, "click".to_owned())),
        "the script still hears it"
    );
    // The script's own publication agrees: it shows, and the stand-in goes.
    view.update(&mut vcx, |v, cx| {
        v.publish(synthetic(&rig, tree(false)), cx)
    });
    settle(cx, |_| true);
    vcx.run_until_parked();
    assert!(view.read_with(&vcx, |v, _| v.disclosures.is_empty()));
    assert!(!view.read_with(&vcx, |v, _| v.disclosure_open(3)));
    // Opening: the chevron turns now; the content comes with the publication.
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
    assert!(view.read_with(&vcx, |v, _| v.disclosure_open(3)));
    view.update(&mut vcx, |v, cx| v.publish(synthetic(&rig, tree(true)), cx));
    settle(cx, |_| true);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("content").is_some());
    assert!(view.read_with(&vcx, |v, _| v.disclosures.is_empty()));
}

/// A press on a panel's item gives the keyboard to the nearest focusable
/// node holding it, as a click does on the web and in AppKit: the Project
/// panel's cards (pressed, so the press stopped there) left its list without
/// focus, and Space and the arrow keys went to the app instead of the grid.
#[gpui::test]
fn a_press_focuses_the_item_it_lands_in(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    let mut ops = node_ops(
        1,
        0,
        "div",
        json!({"id":"list","role":"listbox","tabIndex":0,"style":{"width":200,"height":80}}),
        &["keydown"],
    );
    ops.extend(node_ops(
        2,
        1,
        "div",
        json!({"id":"card","role":"cell","tabIndex":-1,"style":{"width":60,"height":40}}),
        &["pointerdown"],
    ));
    ops.extend(node_ops(
        3,
        2,
        "div",
        json!({"id":"name","style":{"width":40,"height":10}}),
        &["pointerdown"],
    ));
    let (view, mut vcx) = window(cx, &rig, synthetic(&rig, ops), Default::default());
    vcx.run_until_parked();
    let at = bounds(&mut vcx, "name").center();
    vcx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    let focused = view.update_in(&mut vcx, |v, window, _| v.focused_node(window));
    // The card (tabIndex -1, no control) is not a focus target here; its
    // list is, and the panel's model knows which card was pressed.
    assert_eq!(focused, Some(1), "the list holding the pressed card");
}

/// The viewer's native picture changes independently of its panel tree. A shell
/// reveal must repaint its overlay instead of retaining the loading-era paint.
#[gpui::test]
fn viewer_repaints_through_reveal_without_a_new_panel_publication(cx: &mut TestAppContext) {
    struct RevealingPanel {
        child: Entity<SurfaceView>,
        panel: &'static str,
        opacity: f32,
    }
    impl Render for RevealingPanel {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().opacity(self.opacity)
                .child(crate::builtins::fixed_body_view(self.panel, &self.child))
        }
    }
    let rig = rig("viewer", 1);
    let _ = publication(&rig);
    for panel in ["viewer", "timeline"] {
        let surface = synthetic(&rig, node_ops(
            1, 0, "div", json!({"style":{"width":60,"height":60}}), &[],
        ));
        let runtime = rig.runtime.clone();
        let (root, vcx) = cx.add_window_view(|window, cx| RevealingPanel {
            child: cx.new(|cx| SurfaceView::new(
                runtime, surface, tokens(), Default::default(), window, cx,
            )),
            panel,
            opacity: 0.25,
        });
        let mut vcx = vcx.clone();
        vcx.run_until_parked();
        let child = root.read_with(&vcx, |v, _| v.child.clone());
        let first = child.read_with(&vcx, |v, _| v.renders);
        for opacity in [0.5, 1.0] {
            root.update(&mut vcx, |v, cx| { v.opacity = opacity; cx.notify(); });
            vcx.run_until_parked();
        }
        let after = child.read_with(&vcx, |v, _| v.renders);
        if panel == "viewer" {
            assert!(after > first, "the viewer retained its loading-era overlay paint");
        } else {
            assert_eq!(after, first, "timeline paint should remain reusable");
        }
    }
}


#[gpui::test]
fn render_queue_keyboard_focus_reveals_inline_actions(cx: &mut TestAppContext) {
    let rig = rig("export", 1);
    let wait = |ticket: pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while ticket.try_take().is_none() {
            assert!(Instant::now() < deadline, "export realm did not settle");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    wait(rig.runtime.observe_render_queue(json!([
        {"id":4,"name":"Credits","format":"mp4","path":"/Exports/Credits.mp4","status":"Failed","progress":0.,"error":"Missing media","finishedAt":1794686460}
    ])).unwrap());
    wait(rig.runtime.observe_export(json!({"phase":"rendering","frame":42,"total":100,"elapsed":2.})).unwrap());
    wait(rig.runtime.show("export").unwrap());
    let (view, mut vcx) = window(cx, &rig, publication(&rig), Default::default());
    vcx.run_until_parked();
    vcx.update(|window, _| window.activate());
    vcx.update(|window, cx| view.update(cx, |v, cx| v.focus_semantic("queue.retry.4", window, cx)));
    let revealed = std::cell::Cell::new(false);
    settle(&mut vcx, |cx| {
        for publication in rig.runtime.take_publications() {
            let surface = &publication.surface;
            if let Some(button) = surface.nodes().values().find(|n|n.props.get("id").and_then(Value::as_str) == Some("queue.retry.4")) {
                if button.parent.and_then(|id|surface.node(id)).and_then(|n|n.props.get("style")).and_then(|s|s.get("opacity")).and_then(Value::as_f64) == Some(1.0) { revealed.set(true); }
            }
            view.update(cx, |v,cx|v.publish(publication,cx));
        }
        revealed.get()
    });
    assert!(revealed.get(), "keyboard focus reveals the row's actions");
}
