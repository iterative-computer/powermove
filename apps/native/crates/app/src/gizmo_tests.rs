//! Viewer gizmos: real pointer input through the mounted viewer panel into kernel
//! gestures. The scene is real; the engine overlay is observed at its entry point.
use crate::gizmo::{GizmoSurface, Host, Tool, ViewFrame};
use crate::v4_view::{Natives, SurfaceView};
use crate::v4_view_tests::{
    PROJECT, Rig, bounds, publication, rig, rig_project, settle, tokens, window,
};
use gpui::{
    App, AppContext, Bounds, Entity, Modifiers, MouseButton, Pixels, Point, TestAppContext,
    VisualTestContext, point, px,
};
use pm_engine_service::OverlayLine;
use pm_kernel::{Call, Caller, HistoryCall, Request, SelectionCall};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Default)]
struct Recorder {
    lines: RefCell<Vec<OverlayLine>>,
    reports: RefCell<Vec<String>>,
    tool: Cell<Option<Tool>>,
}
struct Recording(Rc<Recorder>);
impl Host for Recording {
    fn time(&self, _: &App) -> f64 {
        0.5
    }
    fn playing(&self, _: &App) -> bool {
        false
    }
    fn frame(&self, bounds: Bounds<Pixels>, _: &App) -> Option<ViewFrame> {
        Some(ViewFrame {
            origin: bounds.origin + point(px(20.), px(20.)),
            scale: 1.,
            size: [640., 360.],
        })
    }
    fn tool(&self, _: &App) -> Tool {
        self.0.tool.get().unwrap_or(Tool::Select)
    }
    fn overlay(&self, lines: Vec<OverlayLine>, _: &mut App) {
        *self.0.lines.borrow_mut() = lines;
    }
    fn report(&self, message: String, _: &mut App) {
        self.0.reports.borrow_mut().push(message);
    }
}
struct Stage {
    rig: Rig,
    surface: Entity<GizmoSurface>,
    recorder: Rc<Recorder>,
    vcx: VisualTestContext,
    _view: Entity<SurfaceView>,
}
/// One visible square with a real transform at its anchor.
fn project() -> String {
    let channel = |key: &str, v: f64| format!(r#""{key}":{{"v":{v},"kf":[]}}"#);
    let channels = [
        ("opacity", 100.),
        ("position.x", 200.),
        ("position.y", 150.),
        ("scale.x", 100.),
        ("scale.y", 100.),
        ("anchor.x", 0.),
        ("anchor.y", 0.),
        ("rotation", 0.),
    ]
    .map(|(key, v)| channel(key, v))
    .join(",");
    format!(
        r##"{{"id":"integration","name":"Square","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{{}},"comps":{{}},"layers":[{{"id":"l0","type":"shape","name":"Square","from":0,"dur":2,"on":true,"p":{{{channels}}},"d":{{"w":100,"h":100}},"fx":[]}}]}}"##
    )
}
/// Two side-by-side squares for picking.
fn pair() -> String {
    project().replace(
        r#""fx":[]}]}"#,
        r#""fx":[]},{"id":"l1","type":"shape","name":"Other","from":0,"dur":2,"on":true,"p":{"opacity":{"v":100,"kf":[]},"position.x":{"v":450,"kf":[]},"position.y":{"v":150,"kf":[]},"scale.x":{"v":100,"kf":[]},"scale.y":{"v":100,"kf":[]},"rotation":{"v":0,"kf":[]}},"d":{"w":100,"h":100},"fx":[]}]}"#,
    )
}
fn stage(cx: &mut TestAppContext) -> Stage {
    stage_on(cx, &project())
}
fn stage_on(cx: &mut TestAppContext, project: &str) -> Stage {
    let rig = rig_project("viewer", project);
    rig.kernel
        .call(
            PROJECT,
            Request::new(
                Caller::native(),
                SelectionCall::Layers {
                    ids: vec!["l0".into()],
                    add: false,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let first = publication(&rig);
    let natives = Natives::default();
    let recorder = Rc::new(Recorder::default());
    let host = Rc::new(Recording(recorder.clone()));
    let kernel = rig.kernel.clone();
    let surface = cx.new(|_| GizmoSurface::new(kernel, PROJECT.into(), tokens(), host));
    natives
        .views
        .borrow_mut()
        .insert("viewer.gizmos".into(), surface.clone().into());
    let (view, vcx) = window(cx, &rig, first, natives);
    let mut stage = Stage {
        rig,
        surface,
        recorder,
        vcx,
        _view: view,
    };
    stage.refresh(cx);
    stage
}
impl Stage {
    fn refresh(&mut self, cx: &mut TestAppContext) {
        self.surface.update(&mut self.vcx, |s, cx| s.refresh(cx));
        let surface = self.surface.clone();
        settle(cx, move |cx| {
            surface.read_with(cx, |s, _| s.idle() && !s.poses().is_empty())
        });
    }
    fn point_of(&mut self, comp: [f64; 2]) -> Point<Pixels> {
        let origin = bounds(&mut self.vcx, "viewer.gizmos").origin;
        origin + point(px(20. + comp[0] as f32), px(20. + comp[1] as f32))
    }
    fn drag(
        &mut self,
        cx: &mut TestAppContext,
        from: Point<Pixels>,
        by: Point<Pixels>,
        m: Modifiers,
    ) {
        let surface = self.surface.clone();
        self.vcx.simulate_mouse_down(from, MouseButton::Left, m);
        for step in 1..=3 {
            let at = from + point(by.x * (step as f32 / 3.), by.y * (step as f32 / 3.));
            self.vcx.simulate_mouse_move(at, MouseButton::Left, m);
            let s = surface.clone();
            settle(cx, move |cx| s.read_with(cx, |s, _| s.idle()));
        }
        self.vcx.simulate_mouse_up(from + by, MouseButton::Left, m);
        let s = surface.clone();
        settle(cx, move |cx| s.read_with(cx, |s, _| s.idle()));
        assert!(
            self.recorder.reports.borrow().is_empty(),
            "{:?}",
            self.recorder.reports.borrow()
        );
        self.refresh(cx);
    }
    fn undo(&mut self, cx: &mut TestAppContext) {
        self.rig
            .kernel
            .call(
                PROJECT,
                Request::new(Caller::native(), Call::History(HistoryCall::Undo)),
            )
            .wait()
            .unwrap()
            .unwrap();
        self.refresh(cx);
    }
    fn pose(&self) -> crate::gizmo::Pose {
        self.surface
            .read_with(&self.vcx, |s, _| s.poses()[0].clone())
    }
    fn points(&self) -> ([f64; 2], [f64; 2], [f64; 2]) {
        self.surface
            .read_with(&self.vcx, |s, _| s.points().unwrap())
    }
}
#[gpui::test]
fn the_engine_receives_a_box_and_handles_for_the_selection(cx: &mut TestAppContext) {
    let s = stage(cx);
    let lines = s.recorder.lines.borrow();
    assert!(lines.len() >= 4 + 8 * 4, "{} lines", lines.len());
    assert!(lines.iter().all(|l| l.ends.iter().all(|v| v.is_finite())));
}
#[gpui::test]
fn dragging_the_body_moves_the_layer_and_undo_restores_it(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let before = s.pose();
    let (body, ..) = s.points();
    let at = s.point_of([body[0] + 30., body[1] + 30.]);
    s.drag(cx, at, point(px(40.), px(-10.)), Modifiers::none());
    let after = s.pose();
    assert_eq!(after.position[0], before.position[0] + 40.);
    assert_eq!(after.position[1], before.position[1] - 10.);
    s.undo(cx);
    assert_eq!(s.pose().position, before.position);
}
#[gpui::test]
fn dragging_a_corner_scales_and_undo_restores_it(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let before = s.pose();
    let (_, corner, _) = s.points();
    let at = s.point_of(corner);
    s.drag(cx, at, point(px(50.), px(50.)), Modifiers::none());
    let after = s.pose();
    assert!(after.scale[0] > before.scale[0], "{:?}", after.scale);
    assert!(after.scale[1] > before.scale[1]);
    s.undo(cx);
    assert_eq!(s.pose().scale, before.scale);
}
#[gpui::test]
fn the_rotate_handle_turns_about_the_anchor_and_undo_restores_it(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let before = s.pose();
    let (.., rotate) = s.points();
    let at = s.point_of(rotate);
    s.drag(cx, at, point(px(60.), px(20.)), Modifiers::none());
    assert!(s.pose().rotation > before.rotation, "{}", s.pose().rotation);
    s.undo(cx);
    assert_eq!(s.pose().rotation, before.rotation);
}
#[gpui::test]
fn the_anchor_tool_moves_the_anchor_and_keeps_the_picture(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    s.recorder.tool.set(Some(Tool::Anchor));
    let before = s.pose();
    let at = s.point_of(before.anchor);
    s.drag(cx, at, point(px(16.), px(12.)), Modifiers::none());
    let after = s.pose();
    assert_eq!(after.local_anchor[0], before.local_anchor[0] + 16.);
    assert_eq!(after.position[0], before.position[0] + 16.);
    for (a, b) in after.corners.iter().zip(before.corners.iter()) {
        assert!((a[0] - b[0]).abs() < 1e-6, "{a:?} {b:?}");
    }
    s.undo(cx);
    assert_eq!(s.pose().local_anchor, before.local_anchor);
}
#[gpui::test]
fn the_opposite_corner_stays_fixed_while_scaling(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let before = s.pose();
    let (_, corner, _) = s.points();
    let at = s.point_of(corner);
    s.drag(cx, at, point(px(50.), px(30.)), Modifiers::none());
    let after = s.pose();
    assert_eq!(after.scale, [150., 130.]);
    assert_eq!(after.corners[0], before.corners[0]);
}
#[gpui::test]
fn option_scales_from_the_centre_and_shift_keeps_the_aspect(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let before = s.pose();
    let (_, corner, _) = s.points();
    let at = s.point_of(corner);
    let both = Modifiers {
        alt: true,
        shift: true,
        ..Default::default()
    };
    s.drag(cx, at, point(px(50.), px(10.)), both);
    let after = s.pose();
    assert_eq!(after.scale[0], after.scale[1]);
    assert!(after.scale[0] > before.scale[0]);
    assert_eq!(after.position, before.position);
}
#[gpui::test]
fn shift_locks_a_move_to_its_dominant_axis(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let before = s.pose();
    let at = s.point_of([210., 160.]);
    s.drag(cx, at, point(px(40.), px(10.)), Modifiers::shift());
    let after = s.pose();
    assert_eq!(
        after.position,
        [before.position[0] + 40., before.position[1]]
    );
}
#[gpui::test]
fn shift_rotates_in_fifteen_degree_steps(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let (.., rotate) = s.points();
    let at = s.point_of(rotate);
    s.drag(cx, at, point(px(60.), px(20.)), Modifiers::shift());
    assert_eq!(s.pose().rotation.rem_euclid(15.), 0.);
    assert_ne!(s.pose().rotation, 0.);
}
#[gpui::test]
fn option_moves_the_anchor_and_lets_the_picture_shift(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    s.recorder.tool.set(Some(Tool::Anchor));
    let before = s.pose();
    let at = s.point_of(before.anchor);
    s.drag(cx, at, point(px(16.), px(12.)), Modifiers::alt());
    let after = s.pose();
    assert_eq!(after.local_anchor, [16., 12.]);
    assert_eq!(after.position, before.position);
    assert_eq!(after.corners[0][0], before.corners[0][0] - 16.);
}
#[gpui::test]
fn pressing_another_layer_selects_it_and_the_same_drag_moves_it(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &pair());
    let at = s.point_of([460., 160.]);
    s.drag(cx, at, point(px(-30.), px(0.)), Modifiers::none());
    assert_eq!(s.pose().id, "l1");
    assert_eq!(s.pose().position, [420., 150.]);
    let selected = s
        .rig
        .kernel
        .published(PROJECT)
        .unwrap()
        .selection
        .layers()
        .to_vec();
    assert_eq!(selected, vec!["l1".to_string()]);
}
#[gpui::test]
fn a_click_on_empty_canvas_clears_the_selection(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let at = s.point_of([20., 20.]);
    s.vcx
        .simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    s.vcx
        .simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel
            .published(PROJECT)
            .unwrap()
            .selection
            .layers()
            .is_empty()
    });
}
#[gpui::test]
fn escape_during_a_move_restores_through_the_kernel(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let before = s.pose();
    let at = s.point_of([210., 160.]);
    s.vcx
        .simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    s.vcx.simulate_mouse_move(
        at + point(px(30.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    let surface = s.surface.clone();
    let idle = surface.clone();
    settle(cx, move |cx| idle.read_with(cx, |g, _| g.idle()));
    assert!(surface.read_with(&s.vcx, |g, _| g.gesture_active()));
    assert!(surface.update(&mut s.vcx, |g, cx| g.cancel(cx)));
    let idle = surface.clone();
    settle(cx, move |cx| idle.read_with(cx, |g, _| g.idle()));
    s.refresh(cx);
    assert_eq!(s.pose().position, before.position);
}
/// The Resolution pop-up reaches the value that sizes the engine's drawable.
#[gpui::test]
fn the_resolution_popup_changes_the_render_scale(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let first = publication(&rig);
    let (view, mut vcx) = window(cx, &rig, first, Natives::default());
    vcx.run_until_parked();
    let scale =
        |rig: &Rig| crate::viewer2::preview_scale(&rig.kernel.published(PROJECT).unwrap().ui_state);
    assert_eq!(scale(&rig), 1.);
    let popup = view
        .read_with(&vcx, |v, _| v.semantic_id_starting("viewer.quality"))
        .expect("Resolution pop-up");
    view.update(&mut vcx, |v, cx| {
        v.change_semantic(&popup, serde_json::json!("0.5"), cx)
    });
    let check = rig.kernel.clone();
    settle(cx, move |_| {
        crate::viewer2::preview_scale(&check.published(PROJECT).unwrap().ui_state) == 0.5
    });
    assert_eq!(
        pm_shell::preview_drawable((1000., 500.), 2., scale(&rig)),
        (1000., 500.)
    );
}
