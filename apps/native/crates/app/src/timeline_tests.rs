//! Timeline gestures: real GPUI input, through the mounted v4 surface and the
//! native timeline element, into the real kernel. Undo reverts each edit.
use crate::timeline_surface::{Delegate, TimelineSurface};
use crate::v4_view::{Natives, SurfaceView};
use crate::v4_view_tests::{PROJECT, Rig, bounds, publication, settle, tokens, window};
use gpui::{
    App, AppContext, Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point,
    ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, Window, point, px,
};
use pm_kernel::{Call, Caller, HistoryCall, Request};
use pm_panel_timeline::HostEffect;
use std::{cell::RefCell, rc::Rc, time::Duration};

#[derive(Default)]
pub(crate) struct Recorder {
    pub time: RefCell<f64>,
    pub seeks: RefCell<Vec<f64>>,
    pub effects: RefCell<Vec<HostEffect>>,
    pub reports: RefCell<Vec<String>>,
    pub commands: RefCell<Vec<String>>,
    pub menus: RefCell<Vec<Vec<serde_json::Value>>>,
    pub choice: RefCell<Option<String>>,
}
pub(crate) struct Recording(pub Rc<Recorder>);
impl Delegate for Recording {
    fn time(&self, _: &App) -> f64 {
        *self.0.time.borrow()
    }
    fn seek(&self, seconds: f64, _: &mut App) {
        *self.0.time.borrow_mut() = seconds;
        self.0.seeks.borrow_mut().push(seconds);
    }
    fn effect(&self, effect: HostEffect, _: &mut App) {
        self.0.effects.borrow_mut().push(effect);
    }
    fn report(&self, message: String, _: &mut App) {
        self.0.reports.borrow_mut().push(message);
    }
    fn command(&self, name: &str, _: &mut App) {
        self.0.commands.borrow_mut().push(name.to_owned());
    }
    fn menu(
        &self,
        items: Vec<serde_json::Value>,
        _: &mut Window,
        cx: &mut App,
        choose: Box<dyn FnOnce(Option<usize>, &mut App)>,
    ) {
        let index = self
            .0
            .choice
            .borrow()
            .as_ref()
            .and_then(|id| items.iter().position(|i| i["id"] == id.as_str()));
        self.0.menus.borrow_mut().push(items);
        cx.defer(move |cx| choose(index, cx));
    }
}
pub(crate) struct Stage {
    pub rig: Rig,
    pub view: Entity<SurfaceView>,
    pub surface: Entity<TimelineSurface>,
    pub recorder: Rc<Recorder>,
    pub vcx: VisualTestContext,
}
pub(crate) fn stage(cx: &mut TestAppContext, layers: usize) -> Stage {
    stage_project(cx, crate::v4_view_tests::project_json(layers))
}
pub(crate) fn stage_project(cx: &mut TestAppContext, project: String) -> Stage {
    let rig = crate::v4_view_tests::rig_project("timeline", &project);
    let first = publication(&rig);
    let natives = Natives::default();
    let recorder = Rc::new(Recorder::default());
    let delegate = Rc::new(Recording(recorder.clone()));
    let (kernel, outline) = (rig.kernel.clone(), natives.outline.clone());
    let surface =
        cx.new(|cx| TimelineSurface::new(kernel, PROJECT.into(), tokens(), delegate, outline, cx));
    natives
        .views
        .borrow_mut()
        .insert("timeline".into(), surface.clone().into());
    let outline = natives.outline.clone();
    let (view, vcx) = window(cx, &rig, first, natives);
    outline.bind(view.downgrade());
    let mut stage = Stage {
        rig,
        view,
        surface,
        recorder,
        vcx,
    };
    stage.idle(cx);
    stage
}
impl Stage {
    /// Worker jobs and repaints quiesce, and JS publications reach the host.
    pub fn idle(&mut self, cx: &mut TestAppContext) {
        let surface = self.surface.clone();
        settle(cx, |cx| surface.read_with(cx, |s, _| s.idle()));
        self.vcx.run_until_parked();
        for (_, ticket) in self.rig.runtime.publish() {
            let ticket = ticket.unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while ticket.try_take().is_none() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        for p in self.rig.runtime.take_publications() {
            self.view.update(&mut self.vcx, |v, cx| v.publish(p, cx));
        }
        self.vcx.run_until_parked();
        assert!(self.rig.runtime.take_errors().is_empty());
        self.surface.update(&mut self.vcx, |_, cx| cx.notify());
        self.vcx.run_until_parked();
    }
    pub fn origin(&mut self) -> Point<Pixels> {
        bounds(&mut self.vcx, "timeline:surface").origin
    }
    /// Press, travel in samples, release: as a real pointer would.
    pub fn drag(
        &mut self,
        cx: &mut TestAppContext,
        from: Point<Pixels>,
        to: Point<Pixels>,
        modifiers: Modifiers,
    ) {
        self.vcx
            .simulate_mouse_down(from, MouseButton::Left, modifiers);
        self.idle(cx);
        for step in 1..=4 {
            let t = step as f32 / 4.;
            let at = from + point((to.x - from.x) * t, (to.y - from.y) * t);
            self.vcx
                .simulate_mouse_move(at, MouseButton::Left, modifiers);
            self.idle(cx);
        }
        self.vcx.simulate_mouse_up(to, MouseButton::Left, modifiers);
        self.idle(cx);
    }
    pub fn from_of(&self, id: &str) -> f64 {
        let p = self.rig.kernel.published(PROJECT).unwrap();
        pm_kernel::model::list_layers(p.snapshot.root().unwrap(), None)
            .into_iter()
            .find(|l| l.id == id)
            .unwrap()
            .in_time
    }
    pub fn out_of(&self, id: &str) -> f64 {
        let p = self.rig.kernel.published(PROJECT).unwrap();
        pm_kernel::model::list_layers(p.snapshot.root().unwrap(), None)
            .into_iter()
            .find(|l| l.id == id)
            .unwrap()
            .out_time
    }
    pub fn undo(&mut self, cx: &mut TestAppContext) {
        self.rig
            .kernel
            .call(
                PROJECT,
                Request::new(Caller::native(), Call::History(HistoryCall::Undo)),
            )
            .wait()
            .unwrap()
            .unwrap();
        self.surface.update(&mut self.vcx, |s, cx| s.published(cx));
        self.idle(cx);
    }
    /// Window position of a time on a row, from the frame the host painted.
    pub fn at(&mut self, row: usize, seconds: f64) -> Point<Pixels> {
        let origin = self.origin();
        let surface = self.surface.clone();
        let (frame, scene) =
            surface.read_with(&self.vcx, |s, _| (s.frame().unwrap(), s.scene().unwrap()));
        let v = frame.viewport;
        let t = tokens();
        let id = scene.navigate_first(row).unwrap();
        let y = t.metrics.ruler as f64 + scene.scroll_for(&id).unwrap() - v.scroll_y
            + t.metrics.track_row as f64 / 2.;
        let x = v.layer_column + (seconds - v.scroll_time) * v.pps;
        origin + point(px(x as f32), px(y as f32))
    }
}
fn op_center(s: &mut Stage, prefix: &str) -> Point<Pixels> {
    let origin = s.origin();
    let frame = s.surface.read_with(&s.vcx, |t, _| t.frame().unwrap());
    let op = frame.ops.iter().find(|o| o.id.starts_with(prefix)).unwrap();
    origin
        + point(
            px((op.bounds.x + op.bounds.width / 2.) as f32),
            px((op.bounds.y + op.bounds.height / 2.) as f32),
        )
}
fn keyed(s: &mut Stage, cx: &mut TestAppContext, times: &[f64]) -> Vec<String> {
    let mut ids = vec![];
    for (i, time) in times.iter().enumerate() {
        let id = format!("k{i}");
        s.rig
            .kernel
            .call(
                PROJECT,
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: None,
                        body: pm_kernel::WriteBody::Commands(vec![
                            pm_kernel::KeyframeCommand::Add {
                                layer_id: "l0".into(),
                                channel: "opacity".into(),
                                time: *time,
                                key_id: id.clone(),
                                value: Some(pm_model::Value::from(50. + i as f64)),
                            }
                            .into(),
                        ]),
                        expected_revision: None,
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        ids.push(id);
    }
    s.surface.update(&mut s.vcx, |t, cx| t.published(cx));
    s.idle(cx);
    ids
}
fn key_time(s: &Stage, id: &str) -> f64 {
    s.surface
        .read_with(&s.vcx, |t, _| {
            t.model().index.key(id).map(|(_, _, k)| k.time)
        })
        .unwrap()
}
#[gpui::test]
fn dragging_a_keyframe_moves_it_and_undo_restores_it(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    let ids = keyed(&mut s, cx, &[0.5, 1.]);
    let from = op_center(&mut s, "key:");
    s.drag(cx, from, from + point(px(40.), px(0.)), Modifiers::none());
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    assert!(key_time(&s, &ids[0]) > 0.5, "{}", key_time(&s, &ids[0]));
    s.undo(cx);
    assert_eq!(key_time(&s, &ids[0]), 0.5);
}
#[gpui::test]
fn marquee_selects_keys_without_seeking(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    keyed(&mut s, cx, &[0.5, 1.]);
    let key = op_center(&mut s, "key:");
    let start = s.at(0, 3.);
    s.drag(cx, start, key + point(px(-12.), px(6.)), Modifiers::none());
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    let selected = s
        .rig
        .kernel
        .published(PROJECT)
        .unwrap()
        .selection
        .keys()
        .len();
    assert_eq!(selected, 2);
    assert!(s.recorder.seeks.borrow().is_empty());
}
#[gpui::test]
fn graph_view_drags_a_key_in_value_and_undo_restores_it(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    let ids = keyed(&mut s, cx, &[0.5, 1.5]);
    let from = op_center(&mut s, "key:");
    s.vcx.simulate_click(from, Modifiers::none());
    s.idle(cx);
    s.surface
        .update(&mut s.vcx, |t, cx| t.command("graph", 0., None, cx));
    s.idle(cx);
    let graph = s.surface.read_with(&s.vcx, |t, _| t.graph_ops_for_test());
    let key = graph
        .iter()
        .find(|o| o.id == format!("graph-key:{}", ids[0]))
        .expect("graph key painted")
        .bounds;
    let at = s.origin()
        + point(
            px((key.x + key.width / 2.) as f32),
            px((key.y + key.height / 2.) as f32),
        );
    let value = |s: &Stage, id: &str| {
        s.surface.read_with(&s.vcx, |t, _| {
            t.model().index.key(id).unwrap().2.value.as_f64().unwrap()
        })
    };
    let before = value(&s, &ids[0]);
    s.drag(cx, at, at - point(px(0.), px(40.)), Modifiers::none());
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    assert!(value(&s, &ids[0]) > before);
    s.undo(cx);
    assert_eq!(value(&s, &ids[0]), before);
}
#[gpui::test]
fn a_timeline_value_readout_scrubs_as_one_undo_step(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    let value = |s: &Stage| {
        s.surface.read_with(&s.vcx, |t, _| {
            t.model().index.channel("l0", "opacity").unwrap().value.as_f64().unwrap()
        })
    };
    let before = value(&s);
    let at = s.at(0, 0.5);
    s.surface.update(&mut s.vcx, |t, _| t.press_property("l0".into(), "opacity".into(), at));
    s.vcx.simulate_mouse_move(at + point(px(-20.), px(0.)), MouseButton::Left, Modifiers::none());
    s.idle(cx);
    s.vcx.simulate_mouse_move(at + point(px(-40.), px(0.)), MouseButton::Left, Modifiers::none());
    s.idle(cx);
    s.vcx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    s.idle(cx);
    assert!(s.recorder.reports.borrow().is_empty(), "{:?}", s.recorder.reports.borrow());
    assert_eq!(value(&s), before - 20.);
    s.undo(cx);
    assert_eq!(value(&s), before);
}
#[gpui::test]
fn layer_rows_offer_a_native_menu_that_runs_commands(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    *s.recorder.choice.borrow_mut() = Some("duplicate".into());
    let at = s.origin() + point(px(100.), s.at(1, 0.).y - s.origin().y);
    s.vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Right,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    s.idle(cx);
    assert_eq!(*s.recorder.commands.borrow(), ["duplicate"]);
    assert_eq!(
        s.rig.kernel.published(PROJECT).unwrap().selection.layers().len(),
        1,
        "the row under the pointer becomes the selection"
    );
}
#[gpui::test]
fn the_column_splitter_resizes_and_double_click_resets(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    let column = s.surface.read_with(&s.vcx, |t, _| t.layer_column());
    let y = s.at(1, 0.).y;
    let edge = s.origin() + point(px(column as f32), px(0.));
    let at = point(edge.x, y);
    s.drag(cx, at, at + point(px(60.), px(0.)), Modifiers::none());
    assert_eq!(
        s.surface.read_with(&s.vcx, |t, _| t.layer_column()),
        column + 60.
    );
    let at = at + point(px(60.), px(0.));
    s.vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    s.vcx
        .simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    s.idle(cx);
    assert_eq!(s.surface.read_with(&s.vcx, |t, _| t.layer_column()), column);
}
#[gpui::test]
fn tracks_mode_ripple_roll_and_slide_edit_through_the_kernel(cx: &mut TestAppContext) {
    let mut s = stage_project(cx, crate::v4_view_tests::strips_json());
    s.surface.update(&mut s.vcx, |t, cx| {
        t.command("timeline.mode:tracks", 0., None, cx)
    });
    s.idle(cx);
    let alt = Modifiers {
        alt: true,
        ..Default::default()
    };
    let control = Modifiers {
        control: true,
        ..Default::default()
    };
    // Ripple: Option-drag A's out edge shifts B and C with it.
    let (a_out, b_in) = (s.out_of("a"), s.from_of("b"));
    let from = s.at(0, a_out) - point(px(2.), px(0.));
    s.drag(cx, from, from - point(px(30.), px(0.)), alt);
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    let shifted = b_in - s.from_of("b");
    assert!(shifted > 0., "ripple must move the suffix");
    assert!((a_out - s.out_of("a") - shifted).abs() < 1e-9);
    s.undo(cx);
    assert_eq!(s.from_of("b"), b_in);
    // Roll: Control-drag the A/B cut moves both edges and neither outer bound.
    let (b_in, b_out) = (s.from_of("b"), s.out_of("b"));
    let from = s.at(0, s.out_of("a")) - point(px(2.), px(0.));
    s.drag(cx, from, from - point(px(30.), px(0.)), control);
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    assert!(s.from_of("b") < b_in);
    assert_eq!(s.out_of("a"), s.from_of("b"));
    assert_eq!(s.out_of("b"), b_out);
    s.undo(cx);
    assert_eq!(s.from_of("b"), b_in);
    // Slide: Option-drag B's body keeps its length and the outer bounds.
    let (length, c_out) = (s.out_of("b") - s.from_of("b"), s.out_of("c"));
    let from = s.at(0, s.from_of("b") + length / 2.);
    s.drag(cx, from, from + point(px(30.), px(0.)), alt);
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    assert!(s.from_of("b") > b_in);
    assert!((s.out_of("b") - s.from_of("b") - length).abs() < 1e-9);
    assert_eq!(s.out_of("c"), c_out);
    s.undo(cx);
    assert_eq!(s.from_of("b"), b_in);
}
#[gpui::test]
fn all_rows_mount_and_the_outline_follows_native_scroll_past_the_old_32_row_cap(
    cx: &mut TestAppContext,
) {
    let mut s = stage(cx, 200);
    let total = s
        .surface
        .read_with(&s.vcx, |t, _| t.scene().unwrap().total());
    assert_eq!(total, 200);
    let at = s.at(1, 0.2);
    s.vcx.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), px(-4000.))),
        ..Default::default()
    });
    s.idle(cx);
    let scrolled = s.surface.read_with(&s.vcx, |t, _| t.scroll_y());
    assert_eq!(scrolled, 4000.);
    // The worker pages the JSX outline to the scrolled window on its own.
    let view = s.view.clone();
    let rig_runtime = s.rig.runtime.clone();
    let vcx = &mut s.vcx;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        vcx.run_until_parked();
        for p in rig_runtime.take_publications() {
            view.update(vcx, |v, cx| v.publish(p, cx));
        }
        vcx.run_until_parked();
        if view.read_with(vcx, |v, _| {
            v.semantic_id_starting("timeline:row:[\"layer\",\"l143\"]")
                .is_some()
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "outline never paged");
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[gpui::test]
fn dragging_a_clip_moves_it_through_the_kernel_and_undo_restores_it(cx: &mut TestAppContext) {
    let mut s = stage(cx, 3);
    let before = s.from_of("l1");
    let from = s.at(1, before + 1.);
    let to = from + point(px(120.), px(0.));
    s.drag(cx, from, to, Modifiers::none());
    let pps = s.surface.read_with(&s.vcx, |t, _| t.pps());
    let moved = s.from_of("l1");
    assert!(
        (moved - (before + 120. / pps)).abs() <= 1. / 30.,
        "clip at {moved}, was {before}, pps {pps}"
    );
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    s.undo(cx);
    assert_eq!(s.from_of("l1"), before);
}
#[gpui::test]
fn trimming_a_clip_edge_changes_its_out_point_and_undo_restores_it(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    let end = s.out_of("l0");
    let from = s.at(0, end) - point(px(2.), px(0.));
    s.drag(cx, from, from - point(px(60.), px(0.)), Modifiers::none());
    assert!(s.out_of("l0") < end, "{} vs {end}", s.out_of("l0"));
    s.undo(cx);
    assert_eq!(s.out_of("l0"), end);
}
#[gpui::test]
fn ruler_press_and_drag_scrubs_frame_exact_without_editing(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    let origin = s.origin();
    let m = tokens().metrics;
    let column = s.surface.read_with(&s.vcx, |t, _| t.layer_column()) as f32;
    let y = px(m.ruler / 2.);
    let revision = s.rig.kernel.published(PROJECT).unwrap().revision();
    let at = origin + point(px(column + 100.), y);
    s.drag(cx, at, at + point(px(50.), px(0.)), Modifiers::none());
    let seeks = s.recorder.seeks.borrow().clone();
    assert!(seeks.len() >= 2);
    let pps = s.surface.read_with(&s.vcx, |t, _| t.pps());
    let last = *seeks.last().unwrap();
    assert!((last - 150. / pps).abs() <= 1. / 30.);
    assert_eq!(
        (last * 30.)
            .fract()
            .abs()
            .min(1. - (last * 30.).fract().abs())
            < 1e-6,
        true
    );
    assert_eq!(
        s.rig.kernel.published(PROJECT).unwrap().revision(),
        revision
    );
}
#[gpui::test]
fn option_wheel_and_pinch_zoom_around_the_pointer(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    let at = s.at(0, 1.);
    let before = s.surface.read_with(&s.vcx, |t, _| t.pps());
    s.vcx.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), px(100.))),
        modifiers: Modifiers {
            alt: true,
            ..Default::default()
        },
        ..Default::default()
    });
    s.idle(cx);
    let after = s.surface.read_with(&s.vcx, |t, _| t.pps());
    assert!(after > before);
    assert!(
        (s.at(0, 1.).x - at.x).abs() < px(1.),
        "zoom keeps the time under the pointer"
    );
    s.vcx.simulate_event(gpui::PinchEvent {
        position: at,
        delta: 0.5,
        ..Default::default()
    });
    s.idle(cx);
    assert!(s.surface.read_with(&s.vcx, |t, _| t.pps()) > after);
}
#[gpui::test]
fn right_click_runs_a_native_menu_action_on_the_kernel(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    let origin = s.origin();
    let column = s.surface.read_with(&s.vcx, |t, _| t.layer_column()) as f32;
    let at = origin + point(px(column + 90.), px(tokens().metrics.ruler / 2.));
    *s.recorder.choice.borrow_mut() = Some("marker.add".into());
    s.vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Right,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    s.vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Right,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 1,
    });
    s.idle(cx);
    let markers = |s: &Stage| {
        s.surface
            .read_with(&s.vcx, |t, _| t.model().index.markers.len())
    };
    assert_eq!(markers(&s), 1);
    assert!(
        s.recorder.menus.borrow()[0]
            .iter()
            .any(|i| i["label"] == "Add Marker")
    );
    s.undo(cx);
    assert_eq!(markers(&s), 0);
}
