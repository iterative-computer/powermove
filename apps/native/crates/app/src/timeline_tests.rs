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
    pub skims: RefCell<Vec<Option<f64>>>,
    pub skimming: std::cell::Cell<bool>,
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
    fn skim(&self, seconds: Option<f64>, _: &mut App) {
        self.0.skims.borrow_mut().push(seconds);
    }
    fn skimming(&self, _: &App) -> bool {
        self.0.skimming.get()
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
    let surface = cx.new(|cx| {
        TimelineSurface::new(
            kernel,
            PROJECT.into(),
            tokens(),
            delegate,
            outline,
            Default::default(),
            cx,
        )
    });
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
        let errors = self.rig.runtime.take_errors();
        assert!(errors.is_empty(), "{errors:?}");
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
    pub fn start_of(&self, id: &str) -> f64 {
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
        .update(&mut s.vcx, |t, cx| t.command("graph", 0., None, false, cx));
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
/// Selects the first key and opens the Value Graph on it.
fn graph_on_first_key(s: &mut Stage, cx: &mut TestAppContext) -> Vec<String> {
    let ids = keyed(s, cx, &[0.5, 1.5]);
    let from = op_center(s, "key:");
    s.vcx.simulate_click(from, Modifiers::none());
    s.idle(cx);
    s.surface
        .update(&mut s.vcx, |t, cx| t.command("graph", 0., None, false, cx));
    s.idle(cx);
    ids
}
#[gpui::test]
fn dragging_a_graph_handle_shapes_the_curve_and_undo_restores_it(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    let ids = graph_on_first_key(&mut s, cx);
    let graph = s.surface.read_with(&s.vcx, |t, _| t.graph_ops_for_test());
    let handle = graph
        .iter()
        .find(|o| o.id == format!("graph-handle:{}:out", ids[0]))
        .expect("the selected key shows its out handle")
        .bounds;
    assert!(
        !graph
            .iter()
            .any(|o| o.id.starts_with(&format!("graph-handle:{}:", ids[1]))),
        "an unselected key shows no handles"
    );
    let at = s.origin()
        + point(
            px((handle.x + handle.width / 2.) as f32),
            px((handle.y + handle.height / 2.) as f32),
        );
    let ease = |s: &Stage| {
        s.surface.read_with(&s.vcx, |t, _| {
            t.model()
                .index
                .key(&ids[0])
                .unwrap()
                .2
                .stored
                .to_value()
                .get("outEase")
                .cloned()
        })
    };
    let before = ease(&s);
    s.drag(cx, at, at - point(px(0.), px(30.)), Modifiers::none());
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    let after = ease(&s);
    assert!(
        after.is_some() && after != before,
        "{before:?} -> {after:?}"
    );
    s.undo(cx);
    assert_eq!(ease(&s), before);
}
#[gpui::test]
fn the_speed_graph_plots_the_rate_of_change(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    graph_on_first_key(&mut s, cx);
    let keys = |s: &Stage| {
        let graph = s.surface.read_with(&s.vcx, |t, _| t.graph()).unwrap();
        let curve = &graph.curves()[0];
        let handles = curve.points.iter().map(|p| p.handles.len()).sum::<usize>();
        (
            curve
                .points
                .iter()
                .map(|p| p.point.value)
                .collect::<Vec<_>>(),
            handles,
        )
    };
    let (values, handles) = keys(&s);
    assert_eq!(values, vec![50., 51.]);
    assert!(handles > 0);
    s.surface.update(&mut s.vcx, |t, cx| {
        t.command("graph.speed", 0., None, false, cx)
    });
    s.idle(cx);
    // 50 to 51 over one second, linear: one unit per second at both keys.
    let (speeds, handles) = keys(&s);
    assert!(speeds.iter().all(|v| (v - 1.).abs() < 1e-3), "{speeds:?}");
    assert_eq!(handles, 0, "speed handles are not value handles");
}
#[gpui::test]
fn a_timeline_value_readout_scrubs_as_one_undo_step(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    let value = |s: &Stage| {
        s.surface.read_with(&s.vcx, |t, _| {
            t.model()
                .index
                .channel("l0", "opacity")
                .unwrap()
                .value
                .as_f64()
                .unwrap()
        })
    };
    let before = value(&s);
    let at = s.at(0, 0.5);
    s.surface.update(&mut s.vcx, |t, _| {
        t.press_property("l0".into(), "opacity".into(), at)
    });
    s.vcx.simulate_mouse_move(
        at + point(px(-20.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    s.idle(cx);
    s.vcx.simulate_mouse_move(
        at + point(px(-40.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    s.idle(cx);
    s.vcx
        .simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    s.idle(cx);
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
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
        s.rig
            .kernel
            .published(PROJECT)
            .unwrap()
            .selection
            .layers()
            .len(),
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
fn each_timeline_draws_its_own_editors_offline_media(cx: &mut TestAppContext) {
    let mut s = stage_project(cx, crate::v4_view_tests::strips_json());
    let viewport = s
        .surface
        .read_with(&s.vcx, |t, _| t.frame().unwrap().viewport);
    let offline = |surface: &Entity<TimelineSurface>, vcx: &VisualTestContext| {
        let scene = surface.read_with(vcx, |t, _| t.scene().unwrap());
        let frame = scene.prepare(viewport, &tokens()).unwrap();
        frame
            .ops
            .iter()
            .filter_map(|o| o.id.strip_prefix("clip-offline:").map(str::to_owned))
            .collect::<Vec<_>>()
    };
    s.surface.update(&mut s.vcx, |t, cx| {
        t.set_offline_media(["source".to_owned()].into(), cx)
    });
    s.idle(cx);
    assert_eq!(offline(&s.surface, &s.vcx), ["a", "b", "c"]);
    // Another editor on the same project reads its media: its clips stay online.
    let (kernel, outline) = (s.rig.kernel.clone(), Natives::default().outline);
    let delegate = Rc::new(Recording(Rc::default()));
    let other = cx.new(|cx| {
        TimelineSurface::new(
            kernel,
            PROJECT.into(),
            tokens(),
            delegate,
            outline,
            Default::default(),
            cx,
        )
    });
    settle(cx, |cx| other.read_with(cx, |t, _| t.idle()));
    assert!(offline(&other, &s.vcx).is_empty());
    assert_eq!(offline(&s.surface, &s.vcx), ["a", "b", "c"]);
    s.surface.update(&mut s.vcx, |t, cx| {
        t.set_offline_media(Default::default(), cx)
    });
    s.idle(cx);
    assert!(offline(&s.surface, &s.vcx).is_empty());
}
#[gpui::test]
fn tracks_mode_ripple_roll_and_slide_edit_through_the_kernel(cx: &mut TestAppContext) {
    let mut s = stage_project(cx, crate::v4_view_tests::strips_json());
    s.surface.update(&mut s.vcx, |t, cx| {
        t.command("timeline.mode:tracks", 0., None, false, cx)
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
    let (a_out, b_in) = (s.out_of("a"), s.start_of("b"));
    let from = s.at(0, a_out) - point(px(2.), px(0.));
    s.drag(cx, from, from - point(px(30.), px(0.)), alt);
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    let shifted = b_in - s.start_of("b");
    assert!(shifted > 0., "ripple must move the suffix");
    assert!((a_out - s.out_of("a") - shifted).abs() < 1e-9);
    s.undo(cx);
    assert_eq!(s.start_of("b"), b_in);
    // Roll: Control-drag the A/B cut moves both edges and neither outer bound.
    let (b_in, b_out) = (s.start_of("b"), s.out_of("b"));
    let from = s.at(0, s.out_of("a")) - point(px(2.), px(0.));
    s.drag(cx, from, from - point(px(30.), px(0.)), control);
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    assert!(s.start_of("b") < b_in);
    assert_eq!(s.out_of("a"), s.start_of("b"));
    assert_eq!(s.out_of("b"), b_out);
    s.undo(cx);
    assert_eq!(s.start_of("b"), b_in);
    // Slide: Option-drag B's body keeps its length and the outer bounds.
    let (length, c_out) = (s.out_of("b") - s.start_of("b"), s.out_of("c"));
    let from = s.at(0, s.start_of("b") + length / 2.);
    s.drag(cx, from, from + point(px(30.), px(0.)), alt);
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    assert!(s.start_of("b") > b_in);
    assert!((s.out_of("b") - s.start_of("b") - length).abs() < 1e-9);
    assert_eq!(s.out_of("c"), c_out);
    s.undo(cx);
    assert_eq!(s.start_of("b"), b_in);
}
#[gpui::test]
fn a_marquee_dragged_past_the_panel_selects_the_rows_it_covers_off_screen(cx: &mut TestAppContext) {
    // Eighty layers starting together, each with a key at 0.5 s.
    let project = (0..80).fold(crate::v4_view_tests::project_json(80), |p, i| {
        p.replace(&format!("\"from\":{},", i as f64 * 0.5), "\"from\":0,")
    });
    let mut s = stage_project(cx, project);
    let commands = (0..80)
        .map(|i| {
            pm_kernel::KeyframeCommand::Add {
                layer_id: format!("l{i}"),
                channel: "opacity".into(),
                time: 0.5,
                key_id: format!("k{i}"),
                value: Some(pm_model::Value::from(50.)),
            }
            .into()
        })
        .collect();
    s.rig
        .kernel
        .call(
            PROJECT,
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: None,
                    body: pm_kernel::WriteBody::Commands(commands),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    s.surface.update(&mut s.vcx, |t, cx| t.published(cx));
    s.idle(cx);
    // Down past the panel's bottom edge to just above layer l61's row: the
    // frame holds only the rows on screen, and l60 is far below them.
    let (frame, scene) = s
        .surface
        .read_with(&s.vcx, |t, _| (t.frame().unwrap(), t.scene().unwrap()));
    let row = |layer: &str| {
        let id = pm_panel_timeline::v4::row_id(&pm_panel_timeline::RowId::Layer(layer.into()));
        tokens().metrics.ruler as f64 + scene.scroll_for(&id).unwrap() - frame.viewport.scroll_y
    };
    assert!(row("l60") > frame.viewport.height);
    let key = op_center(&mut s, "key:");
    let start = s.at(0, 3.);
    let end = point(key.x - px(12.), s.origin().y + px(row("l61") as f32 - 2.));
    s.drag(cx, start, end, Modifiers::none());
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
        .to_vec();
    assert_eq!(selected.len(), 61, "{selected:?}");
    assert!(selected.iter().any(|k| k == "k60"), "{selected:?}");
    assert!(!selected.iter().any(|k| k == "k61"), "{selected:?}");
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
    let before = s.start_of("l1");
    let from = s.at(1, before + 1.);
    let to = from + point(px(120.), px(0.));
    s.drag(cx, from, to, Modifiers::none());
    let pps = s.surface.read_with(&s.vcx, |t, _| t.pps());
    let moved = s.start_of("l1");
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
    assert_eq!(s.start_of("l1"), before);
}
#[gpui::test]
fn a_shift_drag_crossing_another_clips_edge_holds_on_it(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    let target = s.out_of("l0");
    let before = s.start_of("l1");
    let pps = s.surface.read_with(&s.vcx, |t, _| t.pps());
    let from = s.at(1, before + 1.);
    // Snapping latches on crossing, then holds through the release radius.
    let travel = ((target - before) * pps) as f32 + 3.;
    s.drag(
        cx,
        from,
        from + point(px(travel), px(0.)),
        Modifiers::shift(),
    );
    assert_eq!(s.start_of("l1"), target);
    s.undo(cx);
    assert_eq!(s.start_of("l1"), before);
}
#[gpui::test]
fn double_clicking_a_name_renames_through_the_kernel_and_escape_cancels(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    let name = |s: &Stage| {
        let p = s.rig.kernel.published(PROJECT).unwrap();
        pm_kernel::model::list_layers(p.snapshot.root().unwrap(), None)[0]
            .name
            .clone()
    };
    let edit = |s: &mut Stage, cx: &mut TestAppContext, value: serde_json::Value| {
        let row = s
            .view
            .read_with(&s.vcx, |v, _| v.id_of_text("Layer 0"))
            .unwrap();
        s.view.update(&mut s.vcx, |v, cx| {
            v.send(row, "doubleclick", Default::default(), cx);
        });
        s.idle(cx);
        s.idle(cx);
        let field = s
            .view
            .read_with(&s.vcx, |v, _| v.cancellable_field())
            .expect("name field");
        s.view
            .update(&mut s.vcx, |v, cx| v.change(field, value, cx));
        s.idle(cx);
        s.idle(cx);
        assert!(
            s.view
                .read_with(&s.vcx, |v, _| v.cancellable_field())
                .is_none()
        );
    };
    edit(&mut s, cx, serde_json::json!(""));
    assert_eq!(name(&s), "Layer 0");
    edit(&mut s, cx, serde_json::json!("Headline"));
    assert_eq!(name(&s), "Headline");
    s.undo(cx);
    assert_eq!(name(&s), "Layer 0");
}
/// A platform request the panel sent, answered as the app would.
fn answer(
    s: &Stage,
    operation: &str,
    value: impl Fn(&serde_json::Value) -> serde_json::Value,
) -> serde_json::Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let requests = s.rig.runtime.take_platform_requests();
        if let Some(request) = requests.into_iter().find(|r| r.operation == operation) {
            s.rig
                .runtime
                .platform_reply(request.id, Ok(value(&request.input)));
            return request.input;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no {operation} request"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[gpui::test]
fn the_time_field_seeks_to_typed_timecode(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    let time = s
        .view
        .read_with(&s.vcx, |v, _| v.semantic_id_starting("timeline:time"))
        .unwrap();
    s.view
        .update(&mut s.vcx, |v, cx| v.click_semantic(&time, cx));
    s.idle(cx);
    s.idle(cx);
    let field = s
        .view
        .read_with(&s.vcx, |v, _| v.cancellable_field())
        .expect("time field");
    s.view.update(&mut s.vcx, |v, cx| {
        v.change(field, serde_json::json!("2:15"), cx)
    });
    cx.run_until_parked();
    let input = answer(&s, "timeline.effect", |_| serde_json::Value::Null);
    assert_eq!(input["kind"], "seek");
    assert_eq!(input["time"], 2.5);
    s.idle(cx);
    assert!(
        s.view
            .read_with(&s.vcx, |v, _| v.cancellable_field())
            .is_none()
    );
}
#[gpui::test]
fn the_parent_button_opens_a_menu_and_parents_through_the_kernel(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    let select = pm_kernel::SelectionCall::Layers {
        ids: vec!["l0".into()],
        add: false,
    };
    s.rig
        .kernel
        .call(PROJECT, Request::new(Caller::native(), select))
        .wait()
        .unwrap()
        .unwrap();
    s.idle(cx);
    let row = s.view.read_with(&s.vcx, |v, _| {
        v.semantic_ids()
            .into_iter()
            .find(|id| id.starts_with("timeline:parent:") && id.contains("l0"))
    });
    let row = row.expect("parent switch");
    s.view
        .update(&mut s.vcx, |v, cx| v.click_semantic(&row, cx));
    cx.run_until_parked();
    let rows = answer(&s, "contextMenu", |_| serde_json::json!("l1"));
    assert_eq!(rows[0]["label"], "None");
    s.idle(cx);
    s.idle(cx);
    let parent = |s: &Stage| {
        let p = s.rig.kernel.published(PROJECT).unwrap();
        pm_kernel::model::list_layers(p.snapshot.root().unwrap(), None)
            .into_iter()
            .find(|l| l.id == "l0")
            .unwrap()
            .parent
    };
    assert_eq!(parent(&s).as_deref(), Some("l1"));
    s.undo(cx);
    assert_eq!(parent(&s), None);
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
    let off_frame = (last * 30.).fract().abs();
    assert!(off_frame.min(1. - off_frame) < 1e-6);
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
/// Wheel scrolling moves the view at once and writes it once, after it rests:
/// every write publishes to all panels.
#[gpui::test]
fn a_wheel_burst_writes_the_view_once_it_rests(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    let at = s.at(0, 1.);
    let stored = |s: &Stage| {
        let state = s.rig.kernel.state();
        state
            .ui
            .get(PROJECT)
            .and_then(|ui| ui.timeline.get("nativeScrollTime"))
            .and_then(pm_model::Value::as_f64)
    };
    let before = stored(&s);
    for _ in 0..5 {
        s.vcx.simulate_event(ScrollWheelEvent {
            position: at,
            delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
            modifiers: Modifiers {
                shift: true,
                ..Default::default()
            },
            ..Default::default()
        });
    }
    s.idle(cx);
    let shown = s.surface.read_with(&s.vcx, |t, _| t.scroll_time());
    assert!(shown > 0., "the view moved with the wheel");
    assert_eq!(stored(&s), before, "nothing is written mid-gesture");
    s.vcx
        .executor()
        .advance_clock(crate::gizmo::NAVIGATION_IDLE * 2);
    s.idle(cx);
    assert_eq!(stored(&s), Some(shown));
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
/// P from the keymap reveals Position under each selected layer, in both the
/// native track rows and the outline beside them.
#[gpui::test]
fn the_reveal_shortcut_shows_property_rows(cx: &mut TestAppContext) {
    let project = r##"{"id":"integration","name":"Synthetic Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"assets":{},"comps":{},"layers":[{"id":"l0","type":"shape","name":"Box","from":0,"dur":2,"on":true,"p":{"opacity":{"v":100,"kf":[]},"position.x":{"v":10,"kf":[]},"position.y":{"v":20,"kf":[]}},"d":{"w":100,"h":100},"fx":[]}]}"##;
    let mut s = stage_project(cx, project.into());
    let rows = |s: &Stage| s.surface.read_with(&s.vcx, |t, _| t.model().rows().len());
    assert_eq!(rows(&s), 1);
    s.surface.update(&mut s.vcx, |t, cx| {
        t.command("timeline.revealProperty:p", 0., None, false, cx)
    });
    s.idle(cx);
    assert_eq!(rows(&s), 3, "Position X and Y rows in the native tracks");
    bounds(&mut s.vcx, r#"timeline:row:["property","l0","position.x"]"#);
}
/// Hovering a layer row shows its switches; leaving hides them again.
#[gpui::test]
fn hovering_a_layer_row_shows_its_switches(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    let lock = r#"timeline:lock:["layer","l1"]"#;
    let painted = |s: &mut Stage| s.vcx.debug_bounds(lock).is_some();
    assert!(!painted(&mut s), "switches stay hidden at rest");
    let row = bounds(&mut s.vcx, r#"timeline:row:["layer","l1"]"#);
    s.vcx
        .simulate_mouse_move(row.center(), None, Modifiers::none());
    s.idle(cx);
    assert!(painted(&mut s), "hover reveals the lock switch");
    s.vcx
        .simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    s.idle(cx);
    assert!(!painted(&mut s), "leaving hides it");
}
/// A click right after the pointer enters a row selects it, although the
/// hover restyles the row before the click lands.
#[gpui::test]
fn clicking_a_layer_row_selects_it(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    let row = bounds(&mut s.vcx, r#"timeline:row:["layer","l1"]"#);
    s.vcx
        .simulate_mouse_move(row.center(), None, Modifiers::none());
    s.vcx.simulate_click(row.center(), Modifiers::none());
    s.idle(cx);
    let selected = s
        .rig
        .kernel
        .published(PROJECT)
        .unwrap()
        .selection
        .layers()
        .to_vec();
    assert_eq!(selected, ["l1".to_string()]);
}
/// Key ids may repeat across layers: pressing the earlier layer's key selects
/// that layer and that key, never the later layer holding the same id.
#[gpui::test]
fn pressing_a_key_id_two_layers_share_selects_the_pressed_layer(cx: &mut TestAppContext) {
    let layer = |id: &str| {
        format!(
            r##"{{"id":"{id}","type":"shape","name":"{id}","from":0,"dur":4,"on":true,"p":{{"opacity":{{"v":100,"kf":[{{"i":"k","t":1,"v":100}}]}}}},"d":{{"w":100,"h":100}},"fx":[]}}"##
        )
    };
    let project = format!(
        r##"{{"id":"integration","name":"Shared","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{{}},"comps":{{}},"layers":[{},{}]}}"##,
        layer("l0"),
        layer("l1")
    );
    let mut s = stage_project(cx, project);
    let row = pm_panel_timeline::v4::row_id(&pm_panel_timeline::RowId::Layer("l0".into()));
    let op = format!("key:{}", serde_json::json!([row, "k"]));
    let at = op_center(&mut s, &op);
    s.vcx.simulate_click(at, Modifiers::none());
    s.idle(cx);
    let selection = s.rig.kernel.published(PROJECT).unwrap().selection.clone();
    assert_eq!(selection.layers(), ["l0".to_string()]);
    assert_eq!(
        selection.key_layers(),
        [("l0".to_string(), "k".to_string())]
    );
}
/// Clips are accessible elements in the tracks list: named, with their times
/// and selection, and an assistive Press selects one through the pointer path.
#[gpui::test]
fn clips_are_accessible_elements_that_a_press_selects(cx: &mut TestAppContext) {
    use gpui::accesskit::{Action, Role};
    let mut s = stage(cx, 2);
    s.vcx.update(|window, _| window.set_a11y_forced(true));
    s.idle(cx);
    let items = |s: &mut Stage| {
        s.vcx.update(|window, _| {
            let tree = window.a11y_tree().unwrap();
            let list = tree
                .nodes
                .iter()
                .find(|(_, n)| n.role() == Role::ListBox && n.label() == Some("Timeline tracks"))
                .map(|(_, n)| n.children().to_vec())
                .expect("the tracks list");
            tree.nodes
                .iter()
                .filter(|(id, n)| list.contains(id) && n.role() == Role::ListBoxOption)
                .map(|(id, n)| {
                    let bounds = window.a11y_node_bounds(*id);
                    (
                        *id,
                        n.label().unwrap_or_default().to_owned(),
                        n.is_selected(),
                        bounds,
                    )
                })
                .collect::<Vec<_>>()
        })
    };
    let clips = items(&mut s);
    assert_eq!(clips.len(), 2, "{clips:?}");
    assert!(clips[0].1.starts_with("Layer 0, "), "{clips:?}");
    assert!(clips[0].1.ends_with(" seconds"), "{clips:?}");
    assert!(clips.iter().all(|c| c.2 != Some(true)));
    // Each element sits on its painted clip, inside the tracks.
    let tracks = bounds(&mut s.vcx, "timeline:surface");
    for (_, _, _, b) in &clips {
        let b = b.expect("bounds");
        assert!(tracks.contains(&b.center()), "{b:?} outside {tracks:?}");
    }
    let target = clips[1].0;
    s.vcx
        .update(|window, cx| window.perform_a11y_action(target, Action::Click, None, cx));
    s.idle(cx);
    let published = s.rig.kernel.published(PROJECT).unwrap();
    let layers: Vec<String> = published
        .selection
        .layers()
        .iter()
        .map(|l| l.to_string())
        .collect();
    assert_eq!(layers, ["l1"]);
    let clips = items(&mut s);
    assert_eq!(clips[1].2, Some(true));
    // A frame that redraws the panel but not the cached tracks view keeps them.
    s.view.update(&mut s.vcx, |_, cx| cx.notify());
    s.vcx.run_until_parked();
    assert_eq!(
        items(&mut s).len(),
        2,
        "a reused view keeps its accessible elements"
    );
}

#[::core::prelude::v1::test]
fn a_long_clip_name_ends_in_an_ellipsis_that_fits() {
    let fits = |room: usize| move |s: &str| s.chars().count() <= room;
    assert_eq!(
        crate::timeline_surface::fit_tail("Interview A", fits(6)).as_deref(),
        Some("Inter…")
    );
    // Trailing spaces never sit before the ellipsis.
    assert_eq!(
        crate::timeline_surface::fit_tail("Interview A", fits(11)).as_deref(),
        Some("Interview…")
    );
    assert_eq!(
        crate::timeline_surface::fit_tail("Ünïcödé", fits(3)).as_deref(),
        Some("Ün…")
    );
    assert_eq!(crate::timeline_surface::fit_tail("Clip", fits(1)), None);
}
/// Final Cut's skimming through the real pointer path: hovering the tracks
/// moves the skimmer (what the viewer shows) and never the playhead;
/// leaving the timeline clears it; a click moves the playhead to it. With
/// skimming off nothing skims.
#[gpui::test]
fn hovering_the_tracks_skims_and_a_click_moves_the_playhead_there(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() <= 1. / 30. + 1e-9);
    let hover = |s: &mut Stage, cx: &mut TestAppContext, at: Point<Pixels>| {
        s.vcx.simulate_mouse_move(at, None, Modifiers::none());
        s.idle(cx);
    };
    // Off: no skimmer.
    let at = s.at(0, 1.0);
    hover(&mut s, cx, at);
    assert!(s.recorder.skims.borrow().is_empty());
    s.recorder.skimming.set(true);
    for seconds in [1.0, 2.0, 1.5] {
        let at = s.at(0, seconds);
        hover(&mut s, cx, at);
        let last = *s.recorder.skims.borrow().last().unwrap();
        assert!(near(last, seconds), "{last:?} for {seconds}");
        let shown = s.surface.read_with(&s.vcx, |t, _| t.skimmer());
        assert_eq!(shown, last, "the ghost playhead is drawn there");
    }
    assert!(s.recorder.seeks.borrow().is_empty(), "skimming never seeks");
    hover(&mut s, cx, point(px(2.), px(2.)));
    assert_eq!(
        *s.recorder.skims.borrow().last().unwrap(),
        None,
        "leaving clears it"
    );
    let at = s.at(1, 2.5);
    hover(&mut s, cx, at);
    s.vcx.simulate_click(at, Modifiers::none());
    s.idle(cx);
    let seeks = s.recorder.seeks.borrow().clone();
    assert!(
        near(seeks.first().copied(), 2.5),
        "a click puts the playhead at the skimmer: {seeks:?}"
    );
}
/// Cmd-Tab (the window stops being active) without moving the pointer
/// clears the skimmer: the viewer returns to the playhead and Space plays
/// from the playhead, not from a stale skimmer (review).
#[gpui::test]
fn deactivating_the_window_clears_the_skimmer(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    s.vcx.update(|window, _| window.activate());
    s.idle(cx);
    s.recorder.skimming.set(true);
    let at = s.at(0, 1.0);
    s.vcx.simulate_mouse_move(at, None, Modifiers::none());
    s.idle(cx);
    let over = |s: &mut Stage| {
        let surface = s.surface.clone();
        s.vcx
            .update(|window, cx| surface.read(cx).pointer_over(window))
    };
    assert!(s.surface.read_with(&s.vcx, |t, _| t.skimmer()).is_some());
    assert!(over(&mut s), "Space would play from the skimmer");
    s.vcx.deactivate_window();
    s.idle(cx);
    assert_eq!(s.surface.read_with(&s.vcx, |t, _| t.skimmer()), None);
    assert_eq!(*s.recorder.skims.borrow().last().unwrap(), None);
    assert!(!over(&mut s), "Space plays from the playhead");
}
