//! Viewer gizmos: real pointer input through the mounted viewer panel into kernel
//! gestures. The scene is real; the engine overlay is observed at its entry point.
use crate::gizmo::{GizmoSurface, Host, Navigate, Tool, ViewFrame, text::TextInput};
use crate::v4_view::{Natives, SurfaceView};
use crate::v4_view_tests::{
    PROJECT, Rig, bounds, publication, rig, rig_project, settle, tokens, window,
};
use gpui::{
    App, AppContext, Bounds, Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    Point, TestAppContext, VisualTestContext, point, px,
};
use gpui_macos::native_text::TextEvent;
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
    navigation: RefCell<Vec<Navigate>>,
    menus: Cell<usize>,
    commands: RefCell<Vec<String>>,
    input: RefCell<Option<Rc<FakeInput>>>,
    variant: RefCell<Option<String>>,
    /// Input times drag steps stamped for the viewer trace.
    stamps: RefCell<Vec<f64>>,
    /// The composition size the viewer reports, when not 640 by 360.
    size: Cell<Option<[f64; 2]>>,
    /// Compositions a double-clicked precomp asked to open.
    opened: RefCell<Vec<String>>,
    /// Model geometry a part pick loads, and the parts it focused.
    assets: RefCell<Option<Arc<dyn pm_scene3d::geometry::AssetGeometry>>>,
    parts: RefCell<Vec<(String, Option<u64>)>>,
}
/// The canvas editor without AppKit: typing is an event, as from NSTextView.
#[derive(Default)]
pub(crate) struct FakeInput {
    value: RefCell<String>,
    selection: Cell<(usize, usize)>,
    events: RefCell<Vec<TextEvent>>,
    wake: RefCell<Option<Rc<dyn Fn()>>>,
    focused: Cell<bool>,
}
impl FakeInput {
    fn send(&self, event: TextEvent) {
        self.events.borrow_mut().push(event);
        let wake = self.wake.borrow().clone();
        if let Some(wake) = wake {
            wake();
        }
    }
    fn type_text(&self, text: &str) {
        self.value.borrow_mut().push_str(text);
        let n = self.value.borrow().encode_utf16().count();
        self.selection.set((n, n));
        self.send(TextEvent::Changed(self.value.borrow().clone()));
    }
    fn key(&self, key: &'static str) {
        self.send(TextEvent::Key(key));
    }
}
impl TextInput for FakeInput {
    fn value(&self) -> String {
        self.value.borrow().clone()
    }
    fn selection(&self) -> (usize, usize) {
        self.selection.get()
    }
    fn set_selection(&self, start: usize, end: usize) {
        let n = self.value.borrow().encode_utf16().count();
        self.selection.set((start.min(n), end.min(n)));
    }
    fn replace_value(&self, value: &str) {
        *self.value.borrow_mut() = value.into();
    }
    fn composing(&self) -> bool {
        false
    }
    fn focused(&self) -> bool {
        self.focused.get()
    }
    fn focus(&self) {
        self.focused.set(true);
    }
    fn set_font(&self, _: &str, _: f64) {}
    fn misspelled(&self) -> Vec<(usize, usize)> {
        vec![]
    }
    fn context_menu(&self, _: (usize, usize)) {}
    fn place(&self, _: Bounds<Pixels>, _: Bounds<Pixels>) {}
    fn stage_caret(&self, _: Option<(Bounds<Pixels>, Bounds<Pixels>)>) {}
    fn stage_input_rect(&self, _: Option<Bounds<Pixels>>) {}
    fn marked_range(&self) -> Option<(usize, usize)> {
        None
    }
    fn take_events(&self) -> Vec<TextEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
    fn on_event(&self, wake: Rc<dyn Fn()>) {
        *self.wake.borrow_mut() = Some(wake);
    }
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
            size: self.0.size.get().unwrap_or([640., 360.]),
        })
    }
    fn tool(&self, _: &App) -> Tool {
        self.0.tool.get().unwrap_or(Tool::Select)
    }
    fn overlay(&self, lines: Vec<OverlayLine>, _: &mut App) {
        *self.0.lines.borrow_mut() = lines;
    }
    fn navigate(&self, change: Navigate, _: &mut App) {
        self.0.navigation.borrow_mut().push(change);
    }
    fn layer_menu(&self, _: Option<String>, _: &mut gpui::Window, _: &mut App) {
        self.0.menus.set(self.0.menus.get() + 1);
    }
    fn report(&self, message: String, _: &mut App) {
        self.0.reports.borrow_mut().push(message);
    }
    fn command(&self, name: &str, _: &mut App) {
        self.0.commands.borrow_mut().push(name.into());
    }
    fn open_composition(&self, comp: String, _: &mut App) {
        self.0.opened.borrow_mut().push(comp);
    }
    fn scene3d_assets(&self, _: &App) -> Option<Arc<dyn pm_scene3d::geometry::AssetGeometry>> {
        self.0.assets.borrow().clone()
    }
    fn focus_part(&self, layer: &str, slot: Option<u64>, _: &mut App) {
        self.0.parts.borrow_mut().push((layer.into(), slot));
    }
    fn input(&self, at: f64, _: &mut App) {
        self.0.stamps.borrow_mut().push(at);
    }
    fn shape_variant(&self, _: &App) -> String {
        self.0
            .variant
            .borrow()
            .clone()
            .unwrap_or_else(|| "rect".into())
    }
    fn text_input(
        &self,
        initial: &str,
        _: &mut gpui::Window,
        _: &mut App,
    ) -> Option<Rc<dyn TextInput>> {
        let input = Rc::new(FakeInput::default());
        *input.value.borrow_mut() = initial.into();
        *self.0.input.borrow_mut() = Some(input.clone());
        Some(input)
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
pub(crate) fn project() -> String {
    project_of("image")
}
fn project_of(kind: &str) -> String {
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
        r##"{{"id":"integration","name":"Square","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{{}},"comps":{{}},"layers":[{{"id":"l0","type":"{kind}","name":"Square","from":0,"dur":2,"on":true,"p":{{{channels}}},"d":{{"w":100,"h":100}},"fx":[]}}]}}"##
    )
}
#[gpui::test]
fn a_corner_resizes_a_shape_instead_of_scaling_it(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &project_of("shape"));
    let before = s.pose();
    let (_, corner, _) = s.points();
    let at = s.point_of(corner);
    s.drag(cx, at, point(px(50.), px(20.)), Modifiers::none());
    let after = s.pose();
    assert_eq!(after.scale, before.scale);
    assert_eq!(after.corners[0], before.corners[0]);
    assert_eq!(
        after.corners[2],
        [before.corners[2][0] + 50., before.corners[2][1] + 20.]
    );
    s.undo(cx);
    assert_eq!(s.pose().corners, before.corners);
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
    let mut stage = unposed(cx, project);
    stage.refresh(cx);
    stage
}
/// A stage that does not wait for a box (headless text has no fonts).
fn unposed(cx: &mut TestAppContext, project: &str) -> Stage {
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
    Stage {
        rig,
        surface,
        recorder,
        vcx,
        _view: view,
    }
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
fn an_untraced_drag_reads_no_input_times(cx: &mut TestAppContext) {
    assert!(!pm_engine_service::trace::enabled());
    let mut s = stage(cx);
    let (body, ..) = s.points();
    let at = s.point_of([body[0] + 30., body[1] + 30.]);
    s.drag(cx, at, point(px(40.), px(-10.)), Modifiers::none());
    assert!(s.recorder.reports.borrow().is_empty());
    assert!(s.recorder.stamps.borrow().is_empty());
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
    assert_eq!(after.scale, [140., 140.], "pictures keep their aspect");
    assert_eq!(after.corners[0], before.corners[0]);
}
#[gpui::test]
fn a_solid_scales_each_axis_freely(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &project_of("solid"));
    let (_, corner, _) = s.points();
    let width = s.pose().corners[1][0] - s.pose().corners[0][0];
    let height = s.pose().corners[3][1] - s.pose().corners[0][1];
    let at = s.point_of([corner[0] - 2., corner[1] - 2.]);
    s.drag(
        cx,
        at,
        point(px(width as f32 / 2.), px(height as f32 / 4.)),
        Modifiers::none(),
    );
    assert_eq!(s.pose().scale, [150., 125.]);
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
/// A tap-to-click (down and up in the same instant) picks like a held press.
#[gpui::test]
fn an_instant_click_on_another_layer_selects_it(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &pair());
    let at = s.point_of([460., 160.]);
    s.vcx
        .simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    s.vcx
        .simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() == ["l1".to_string()]
    });
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
#[gpui::test]
fn the_hand_tool_drags_the_view_without_editing(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    s.recorder.tool.set(Some(Tool::Hand));
    let before = s.pose();
    let at = s.point_of([210., 160.]);
    s.drag(cx, at, point(px(30.), px(-12.)), Modifiers::none());
    let moved: Vec<_> = s.recorder.navigation.borrow().clone();
    assert_eq!(moved.len(), 3);
    let total = moved
        .iter()
        .fold(point(px(0.), px(0.)), |sum, change| match change {
            Navigate::Pan(by) => sum + *by,
            other => panic!("{other:?}"),
        });
    assert_eq!(total, point(px(30.), px(-12.)));
    assert_eq!(s.pose().position, before.position);
}
#[gpui::test]
fn the_zoom_tool_keeps_the_point_under_the_pointer(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    s.recorder.tool.set(Some(Tool::Zoom));
    let at = s.point_of([300., 100.]);
    s.vcx.simulate_click(at, Modifiers::alt());
    let [Navigate::Zoom { factor, .. }] = s.recorder.navigation.borrow().clone()[..] else {
        panic!("one zoom step");
    };
    assert_eq!(factor, 0.8);
    let center = point(px(500.), px(300.));
    let pointer = point(px(700.), px(400.));
    let change = Navigate::Zoom {
        factor: 2.,
        at: pointer,
    };
    let (zoom, pan) = crate::gizmo::navigated(change, 1., [0., 0.], center, 8.);
    assert_eq!(zoom, 2.);
    // 200, 100 from centre now lands 400, 200 out: the view moves back by that.
    assert_eq!(pan, [-200., -100.]);
}
/// The app's state and its delegate in the shape that panicked: the owner holds
/// the surface, updates it during its own update, and the host reads the owner.
struct Owner {
    surface: Option<Entity<GizmoSurface>>,
    overlays: usize,
}
struct OwnerHost(gpui::WeakEntity<Owner>);
impl Host for OwnerHost {
    fn time(&self, cx: &App) -> f64 {
        self.0
            .upgrade()
            .map_or(0., |o| o.read(cx).overlays as f64 * 0.)
    }
    fn playing(&self, cx: &App) -> bool {
        self.0.upgrade().is_some_and(|o| o.read(cx).overlays > 99)
    }
    fn frame(&self, bounds: Bounds<Pixels>, cx: &App) -> Option<ViewFrame> {
        self.0.upgrade()?.read(cx);
        Some(ViewFrame {
            origin: bounds.origin,
            scale: 1.,
            size: [640., 360.],
        })
    }
    fn tool(&self, _: &App) -> Tool {
        Tool::Select
    }
    fn overlay(&self, _: Vec<OverlayLine>, cx: &mut App) {
        let _ = self.0.update(cx, |o, _| o.overlays += 1);
    }
    fn navigate(&self, _: Navigate, _: &mut App) {}
    fn layer_menu(&self, _: Option<String>, _: &mut gpui::Window, _: &mut App) {}
    fn report(&self, _: String, _: &mut App) {}
}
#[gpui::test]
fn refreshing_during_the_owners_update_never_reads_it_while_leased(cx: &mut TestAppContext) {
    let rig = rig_project("viewer", &project());
    let kernel = rig.kernel.clone();
    let owner = cx.new(|_| Owner {
        surface: None,
        overlays: 0,
    });
    let weak = owner.downgrade();
    let surface =
        cx.new(|_| GizmoSurface::new(kernel, PROJECT.into(), tokens(), Rc::new(OwnerHost(weak))));
    owner.update(cx, |o, _| o.surface = Some(surface));
    // Controller::refresh, tick_builtins and toggleLayerControls, each under lease.
    owner.update(cx, |o, cx| {
        let surface = o.surface.clone().unwrap();
        surface.update(cx, |s, cx| s.refresh(cx));
    });
    owner.update(cx, |o, cx| {
        let surface = o.surface.clone().unwrap();
        surface.update(cx, |s, cx| s.toggle_controls(cx));
    });
    let check = owner.clone();
    settle(cx, move |cx| check.read_with(cx, |o, _| o.overlays >= 2));
}
#[gpui::test]
fn zoom_stops_at_what_the_view_state_accepts_and_the_display_can_show(_: &mut TestAppContext) {
    use pm_panel_viewerchrome::Navigation;
    let at = point(px(0.), px(0.));
    let zoom = |limit: f64| {
        let change = Navigate::Zoom { factor: 10., at };
        crate::gizmo::navigated(change, 4., [0., 0.], at, limit).0
    };
    assert_eq!(zoom(100.), 8.);
    let limit = pm_shell::zoom_limit([1920., 1080.], 2.);
    assert!((limit - 2.1333).abs() < 1e-3);
    assert_eq!(zoom(limit), limit);
    let stored = Navigation {
        fit: false,
        zoom: zoom(100.),
        pan: Default::default(),
    };
    assert!(stored.checked().is_ok());
}
/// A hand drag under load: publications of earlier steps land between events.
#[gpui::test]
fn publications_never_undo_navigation_they_are_behind(_: &mut TestAppContext) {
    use pm_panel_viewerchrome::{Navigation, Point as Pan};
    let center = point(px(0.), px(0.));
    let mut view = crate::gizmo::ViewPlacement::default();
    let step = Navigate::Pan(point(px(10.), px(0.)));
    view.navigate(step, center, 8.);
    view.navigate(step, center, 8.);
    let behind = Navigation {
        fit: false,
        zoom: 1.,
        pan: Pan { x: 10., y: 0. },
    };
    view.adopt(&behind);
    view.navigate(step, center, 8.);
    assert_eq!(view.pan, [30., 0.]);
    // Two wheel steps in one frame both count, without a render between them.
    let at = point(px(0.), px(0.));
    let wheel = Navigate::Zoom { factor: 2., at };
    view.navigate(wheel, center, 8.);
    view.navigate(wheel, center, 8.);
    assert_eq!(view.scale.get(), 4.);
    view.written();
    view.adopt(&behind);
    assert_eq!(view.pan, [10., 0.]);
}
/// Hand protection written by a nudge or Center Anchor must not stop the viewer.
#[gpui::test]
fn a_drag_edits_a_channel_a_previous_hand_edit_protected(cx: &mut TestAppContext) {
    let protected = project().replace(
        r#""fx":[]}]}"#,
        r#""fx":[],"locked_intent":{"position.x":"human"}}]}"#,
    );
    let mut s = stage_on(cx, &protected);
    let before = s.pose();
    let (centre, _, _) = s.points();
    let at = s.point_of(centre);
    s.drag(cx, at, point(px(30.), px(0.)), Modifiers::none());
    assert_eq!(s.pose().position[0], before.position[0] + 30.);
    let p = s.rig.kernel.published(PROJECT).unwrap();
    let root = p.snapshot.root().unwrap();
    let layer = pm_kernel::model::find_layer(root, "l0")
        .and_then(|at| pm_kernel::model::layer_at(root, &at))
        .unwrap();
    assert!(
        layer
            .get("locked_intent")
            .and_then(|i| i.get("position.y"))
            .is_some()
    );
}
/// A selected group and its member: the member follows the group, once.
#[gpui::test]
fn dragging_a_group_with_its_member_moves_the_member_once(cx: &mut TestAppContext) {
    let grouped = pair()
        .replace(
            r#""id":"l1","type":"shape""#,
            r#""id":"l1","group":"g","type":"shape""#,
        )
        .replace(
            r#""layers":[{"#,
            r#""layers":[{"id":"g","type":"group","name":"Group","from":0,"dur":2,"on":true,"p":{"position.x":{"v":0,"kf":[]},"position.y":{"v":0,"kf":[]},"scale.x":{"v":100,"kf":[]},"scale.y":{"v":100,"kf":[]},"rotation":{"v":0,"kf":[]},"opacity":{"v":100,"kf":[]}},"d":{},"fx":[]},{"#,
        );
    let mut s = stage_on(cx, &grouped);
    s.rig
        .kernel
        .call(
            PROJECT,
            Request::new(
                Caller::native(),
                SelectionCall::Layers {
                    ids: vec!["g".into(), "l1".into()],
                    add: false,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    s.refresh(cx);
    let at = s.point_of([460., 160.]);
    s.drag(cx, at, point(px(40.), px(0.)), Modifiers::none());
    let p = s.rig.kernel.published(PROJECT).unwrap();
    let mut mirror = pm_kernel::QueryMirror::new(&p);
    let mut x = |id: &str| {
        mirror
            .queries()
            .evaluate(None, id, "position.x", 0.5)
            .unwrap()
            .as_f64()
            .unwrap()
    };
    assert_eq!((x("g"), x("l1")), (40., 450.));
}
#[gpui::test]
fn a_secondary_press_selects_the_layer_and_opens_the_layer_menu(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &pair());
    let at = s.point_of([460., 160.]);
    let (kernel, recorder) = (s.rig.kernel.clone(), s.recorder.clone());
    s.vcx
        .simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    s.vcx
        .simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    settle(cx, move |_| recorder.menus.get() == 1);
    let selected = kernel
        .published(PROJECT)
        .unwrap()
        .selection
        .layers()
        .to_vec();
    assert_eq!(selected, ["l1"]);
}
/// The Resolution pop-up reaches the quality the engine renders the viewer at.
#[gpui::test]
fn the_resolution_popup_changes_the_render_scale(cx: &mut TestAppContext) {
    let rig = rig("viewer", 1);
    let first = publication(&rig);
    let (view, mut vcx) = window(cx, &rig, first, Natives::default());
    vcx.run_until_parked();
    let quality = |rig: &Rig| {
        let ui = rig.kernel.published(PROJECT).unwrap().ui_state.clone();
        crate::viewer2::engine_quality(pm_panel_viewerchrome::Chrome::read(&ui).unwrap().quality)
    };
    assert_eq!(quality(&rig), pm_engine_service::Quality::Auto);
    let popup = view
        .read_with(&vcx, |v, _| v.semantic_id_starting("viewer.quality"))
        .expect("Resolution pop-up");
    view.update(&mut vcx, |v, cx| {
        v.change_semantic(&popup, serde_json::json!("0.5"), cx)
    });
    let check = rig.kernel.clone();
    settle(cx, move |_| {
        let ui = check.published(PROJECT).unwrap().ui_state.clone();
        pm_panel_viewerchrome::Chrome::read(&ui).is_ok_and(|c| {
            crate::viewer2::engine_quality(c.quality) == pm_engine_service::Quality::Half
        })
    });
    assert_eq!(quality(&rig), pm_engine_service::Quality::Half);
}
/// A selected layer the playhead is outside of draws no box (viewer.ts
/// visibleSelectionLayers); the active one keeps its own.
#[gpui::test]
fn a_selected_layer_outside_the_playhead_has_no_box(cx: &mut TestAppContext) {
    let later = pair().replace(
        r#""name":"Other","from":0,"dur":2"#,
        r#""name":"Other","from":1,"dur":1"#,
    );
    let mut s = stage_on(cx, &later);
    let both = SelectionCall::Layers {
        ids: vec!["l0".into(), "l1".into()],
        add: false,
    };
    let call = Request::new(Caller::native(), both);
    s.rig.kernel.call(PROJECT, call).wait().unwrap().unwrap();
    s.refresh(cx);
    let ids: Vec<String> = s.surface.read_with(&s.vcx, |g, _| {
        g.poses().iter().map(|p| p.id.clone()).collect()
    });
    assert_eq!(ids, ["l0".to_string()]);
}
fn layer_value(kernel: &pm_kernel::KernelHandle, id: &str) -> Option<pm_model::Value> {
    let p = kernel.published(PROJECT).unwrap();
    let root = p.snapshot.root().unwrap();
    let at = pm_kernel::model::find_layer(root, id)?;
    Some(pm_kernel::model::layer_at(root, &at)?.to_value())
}
/// A static channel value: `p` of a layer, or of its first mask.
fn channel(layer: &pm_model::Value, mask: bool, key: &str) -> Option<f64> {
    let owner = if mask {
        layer.get("masks")?.as_array()?.first()?
    } else {
        layer
    };
    owner.get("p")?.get(key)?.get("v")?.as_f64()
}
/// Shape tool over a selected shape layer: the drag draws a new shape layer
/// whose Scale is the drawn size, selected, in one undo step.
#[gpui::test]
fn the_shape_tool_draws_a_shape_layer_from_a_drag(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &project_of("shape"));
    s.recorder.tool.set(Some(Tool::Shape));
    let at = s.point_of([300., 200.]);
    s.drag(cx, at, point(px(60.), px(40.)), Modifiers::none());
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() != ["l0"]
    });
    let p = s.rig.kernel.published(PROJECT).unwrap();
    let id = p.selection.layers()[0].clone();
    let layer = layer_value(&s.rig.kernel, &id).unwrap();
    assert_eq!(
        layer.get("type").and_then(pm_model::Value::as_str),
        Some("shape")
    );
    assert_eq!(channel(&layer, false, "position.x"), Some(330.));
    assert_eq!(channel(&layer, false, "position.y"), Some(220.));
    assert_eq!(channel(&layer, false, "scale.x"), Some(60.));
    assert_eq!(channel(&layer, false, "scale.y"), Some(40.));
    let undo = Request::new(Caller::native(), Call::History(HistoryCall::Undo));
    s.rig.kernel.call(PROJECT, undo).wait().unwrap().unwrap();
    assert!(layer_value(&s.rig.kernel, &id).is_none(), "one undo step");
}
/// With a maskable layer selected the drawn box becomes a mask on it.
#[gpui::test]
fn the_shape_tool_draws_a_mask_on_a_selected_image(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    s.recorder.tool.set(Some(Tool::Shape));
    let at = s.point_of([300., 200.]);
    s.drag(cx, at, point(px(60.), px(40.)), Modifiers::none());
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        layer_value(&kernel, "l0").is_some_and(|l| channel(&l, true, "w").is_some())
    });
    let layer = layer_value(&s.rig.kernel, "l0").unwrap();
    assert_eq!(channel(&layer, true, "feather"), Some(0.));
    assert_eq!(channel(&layer, true, "w"), Some(60.));
    assert_eq!(channel(&layer, true, "h"), Some(40.));
    let p = s.rig.kernel.published(PROJECT).unwrap();
    assert_eq!(p.selection.layers(), ["l0".to_string()]);
}

fn text_of(layer: &pm_model::Value) -> Option<String> {
    layer.get("d")?.get("text")?.as_str().map(str::to_owned)
}
/// The Text tool's click makes a layer inside one gesture: typing writes it,
/// Command-Return commits, the layer is named after its text, the tool goes
/// back to Select and one Undo removes it all.
#[gpui::test]
fn the_text_tool_types_a_new_layer_in_one_undo_step(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    s.recorder.tool.set(Some(Tool::Text));
    let at = s.point_of([400., 300.]);
    s.vcx.simulate_click(at, Modifiers::none());
    let recorder = s.recorder.clone();
    settle(cx, move |_| recorder.input.borrow().is_some());
    let input = s.recorder.input.borrow().clone().unwrap();
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() != ["l0"]
    });
    let id = s.rig.kernel.published(PROJECT).unwrap().selection.layers()[0].clone();
    for part in ["Hel", "lo", " there"] {
        input.type_text(part);
        cx.run_until_parked();
    }
    let kernel = s.rig.kernel.clone();
    let probe = id.clone();
    settle(cx, move |_| {
        layer_value(&kernel, &probe)
            .and_then(|l| text_of(&l))
            .as_deref()
            == Some("Hello there")
    });
    input.key("commit");
    let surface = s.surface.clone();
    settle(cx, move |cx| {
        surface.read_with(cx, |g, _| g.text_open().is_none())
    });
    let layer = layer_value(&s.rig.kernel, &id).unwrap();
    assert_eq!(
        layer.get("type").and_then(pm_model::Value::as_str),
        Some("text")
    );
    assert_eq!(
        layer.get("name").and_then(pm_model::Value::as_str),
        Some("Hello there")
    );
    assert_eq!(channel(&layer, false, "position.x"), Some(400.));
    assert_eq!(s.recorder.commands.borrow().as_slice(), ["toolSelect"]);
    let undo = Request::new(Caller::native(), Call::History(HistoryCall::Undo));
    s.rig.kernel.call(PROJECT, undo).wait().unwrap().unwrap();
    assert!(layer_value(&s.rig.kernel, &id).is_none(), "one undo step");
    assert!(layer_value(&s.rig.kernel, "l0").is_some());
}
/// Leaving a fresh layer empty leaves no layer and no history entry.
#[gpui::test]
fn an_empty_new_text_leaves_nothing_behind(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    s.recorder.tool.set(Some(Tool::Text));
    let at = s.point_of([400., 300.]);
    s.vcx.simulate_click(at, Modifiers::none());
    let recorder = s.recorder.clone();
    settle(cx, move |_| recorder.input.borrow().is_some());
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() != ["l0"]
    });
    let id = s.rig.kernel.published(PROJECT).unwrap().selection.layers()[0].clone();
    let input = s.recorder.input.borrow().clone().unwrap();
    input.focused.set(false);
    input.send(TextEvent::Blurred);
    let surface = s.surface.clone();
    settle(cx, move |cx| {
        surface.read_with(cx, |g, _| g.text_open().is_none())
    });
    assert!(layer_value(&s.rig.kernel, &id).is_none());
    let undo = Request::new(Caller::native(), Call::History(HistoryCall::Undo));
    let reply = s
        .rig
        .kernel
        .call(PROJECT, undo)
        .wait()
        .unwrap()
        .unwrap()
        .reply;
    assert!(layer_value(&s.rig.kernel, "l0").is_some(), "{reply:?}");
    assert!(
        !matches!(reply, pm_kernel::Reply::Bool(true)),
        "nothing to undo: {reply:?}"
    );
}
/// Return edits the selected text layer; Escape (a blur) commits the change
/// as one "Edit Text" step that Undo reverts.
#[gpui::test]
fn return_edits_the_selected_text_and_undo_reverts_it(cx: &mut TestAppContext) {
    let mut s = unposed(
        cx,
        &project_of("text").replace(r#""d":{"w":100,"h":100}"#, r#""d":{"text":"Hi"}"#),
    );
    let surface = s.surface.clone();
    surface.update_in(&mut s.vcx, |g, window, cx| {
        assert!(g.edit_selected_text(window, cx))
    });
    let recorder = s.recorder.clone();
    settle(cx, move |_| recorder.input.borrow().is_some());
    let input = s.recorder.input.borrow().clone().unwrap();
    assert_eq!(input.value(), "Hi");
    input.type_text(" all");
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        layer_value(&kernel, "l0")
            .and_then(|l| text_of(&l))
            .as_deref()
            == Some("Hi all")
    });
    input.focused.set(false);
    input.send(TextEvent::Blurred);
    let surface = s.surface.clone();
    settle(cx, move |cx| {
        surface.read_with(cx, |g, _| g.text_open().is_none())
    });
    assert_eq!(
        layer_value(&s.rig.kernel, "l0")
            .and_then(|l| text_of(&l))
            .as_deref(),
        Some("Hi all")
    );
    assert!(
        s.recorder.commands.borrow().is_empty(),
        "the Select tool stays"
    );
    let undo = Request::new(Caller::native(), Call::History(HistoryCall::Undo));
    s.rig.kernel.call(PROJECT, undo).wait().unwrap().unwrap();
    assert_eq!(
        layer_value(&s.rig.kernel, "l0")
            .and_then(|l| text_of(&l))
            .as_deref(),
        Some("Hi")
    );
}

/// The Pen tool on a selected image: clicks draw a mask path (a drag pulls
/// out mirrored tangents), a click on the first vertex closes it, and each
/// press is its own undo step.
#[gpui::test]
fn the_pen_draws_a_closed_mask_path_on_the_selected_layer(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    s.recorder.tool.set(Some(Tool::Pen));
    let press =
        |s: &mut Stage, cx: &mut TestAppContext, at: [f64; 2], by: Option<Point<Pixels>>| {
            let p = s.point_of(at);
            match by {
                Some(by) => s.drag(cx, p, by, Modifiers::none()),
                None => {
                    s.vcx.simulate_click(p, Modifiers::none());
                    let surface = s.surface.clone();
                    settle(cx, move |cx| surface.read_with(cx, |g, _| g.idle()));
                }
            }
            s.surface.update(&mut s.vcx, |g, cx| g.refresh(cx));
        };
    for (vertices, (at, by)) in [
        ([300., 100.], None),
        ([400., 100.], Some(point(px(30.), px(0.)))),
        ([400., 250.], None),
    ]
    .into_iter()
    .enumerate()
    {
        let vertices = vertices + 1;
        press(&mut s, cx, at, by);
        let surface = s.surface.clone();
        settle(cx, move |cx| {
            surface.read_with(cx, |g, _| g.pen_vertices() == vertices)
        });
    }
    press(&mut s, cx, [300., 100.], None);
    let kernel = s.rig.kernel.clone();
    let closed = move |kernel: &pm_kernel::KernelHandle| {
        layer_value(kernel, "l0").and_then(|l| {
            l.get("masks")?
                .as_array()?
                .first()?
                .get("path")?
                .get("p")?
                .get("closed")?
                .get("v")
                .cloned()
        }) == Some(pm_model::Value::Bool(true))
    };
    settle(cx, move |_| closed(&kernel));
    let layer = layer_value(&s.rig.kernel, "l0").unwrap();
    let path = layer.get("masks").unwrap().as_array().unwrap()[0]
        .get("path")
        .unwrap()
        .clone();
    let v = path.get("vertices").unwrap().as_array().unwrap();
    assert_eq!(v.len(), 3);
    let x = |i: usize, k: &str| v[i].get("p")?.get(k)?.get("v")?.as_f64();
    assert_eq!(
        x(0, "x"),
        Some(100.),
        "layer space: the image sits at 200,150 with anchor 0"
    );
    assert_eq!(x(1, "outX"), Some(30.));
    assert_eq!(x(1, "inX"), Some(-30.));
    // Undo reopens the path, then removes the third vertex.
    let undo = || Request::new(Caller::native(), Call::History(HistoryCall::Undo));
    s.rig.kernel.call(PROJECT, undo()).wait().unwrap().unwrap();
    assert!(!closed(&s.rig.kernel));
    s.rig.kernel.call(PROJECT, undo()).wait().unwrap().unwrap();
    let layer = layer_value(&s.rig.kernel, "l0").unwrap();
    let path = layer.get("masks").unwrap().as_array().unwrap()[0]
        .get("path")
        .unwrap()
        .clone();
    assert_eq!(path.get("vertices").unwrap().as_array().unwrap().len(), 2);
}

/// The Star variant draws a star shape layer named for it, even over a
/// maskable selection (only rectangles and ellipses become masks).
#[gpui::test]
fn the_star_variant_draws_a_star_layer(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    s.recorder.tool.set(Some(Tool::Shape));
    *s.recorder.variant.borrow_mut() = Some("star".into());
    let at = s.point_of([300., 200.]);
    s.drag(cx, at, point(px(60.), px(40.)), Modifiers::none());
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() != ["l0"]
    });
    let id = s.rig.kernel.published(PROJECT).unwrap().selection.layers()[0].clone();
    let layer = layer_value(&s.rig.kernel, &id).unwrap();
    assert_eq!(
        layer.get("name").and_then(pm_model::Value::as_str),
        Some("Star")
    );
    assert_eq!(
        layer
            .get("d")
            .and_then(|d| d.get("shape"))
            .and_then(pm_model::Value::as_str),
        Some("star")
    );
}
/// A Select-tool drag from empty canvas selects the layers it encloses;
/// Shift toggles them instead.
#[gpui::test]
fn a_marquee_selects_the_layers_it_encloses(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &pair());
    let clear = SelectionCall::Layers {
        ids: vec![],
        add: false,
    };
    s.rig
        .kernel
        .call(PROJECT, Request::new(Caller::native(), clear))
        .wait()
        .unwrap()
        .unwrap();
    s.surface.update(&mut s.vcx, |g, cx| g.refresh(cx));
    cx.run_until_parked();
    // Around the first square (150..250, 100..200) only.
    let from = s.point_of([120., 80.]);
    s.vcx
        .simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    let surface = s.surface.clone();
    settle(cx, move |cx| surface.read_with(cx, |g, _| g.idle()));
    for step in 1..=4 {
        let at = from + point(px(40. * step as f32), px(35. * step as f32));
        s.vcx
            .simulate_mouse_move(at, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
    }
    s.vcx.simulate_mouse_up(
        from + point(px(160.), px(140.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() == ["l0"]
    });
}
/// A move that brings the box's right edge within 8 points of the
/// composition's centre snaps onto it; Command moves freely.
#[gpui::test]
fn moves_snap_to_the_composition_centre_unless_command_is_held(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    // The square spans 150..250; 250 + 66 lands 4 short of the centre at 320.
    let (centre, _, _) = s.points();
    let at = s.point_of(centre);
    s.drag(cx, at, point(px(66.), px(0.)), Modifiers::none());
    assert_eq!(s.pose().position, [270., 150.]);
    s.undo(cx);
    let at = s.point_of(centre);
    s.drag(cx, at, point(px(66.), px(0.)), Modifiers::command());
    assert_eq!(s.pose().position, [266., 150.]);
}
/// Two selected layers share one box: dragging its corner scales both about
/// the opposite corner, and one Undo restores them.
#[gpui::test]
fn the_common_box_scales_a_multiple_selection(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &pair());
    let both = SelectionCall::Layers {
        ids: vec!["l0".into(), "l1".into()],
        add: false,
    };
    s.rig
        .kernel
        .call(PROJECT, Request::new(Caller::native(), both))
        .wait()
        .unwrap()
        .unwrap();
    s.refresh(cx);
    let surface = s.surface.clone();
    settle(cx, move |cx| {
        surface.read_with(cx, |g, _| g.poses().len() == 2)
    });
    let poses = s.surface.read_with(&s.vcx, |g, _| g.poses());
    let corner = [
        poses
            .iter()
            .flat_map(|p| p.corners)
            .map(|c| c[0])
            .fold(f64::MIN, f64::max),
        poses
            .iter()
            .flat_map(|p| p.corners)
            .map(|c| c[1])
            .fold(f64::MIN, f64::max),
    ];
    let left = poses
        .iter()
        .flat_map(|p| p.corners)
        .map(|c| c[0])
        .fold(f64::MAX, f64::min);
    let top = poses
        .iter()
        .flat_map(|p| p.corners)
        .map(|c| c[1])
        .fold(f64::MAX, f64::min);
    let at = s.point_of(corner);
    let by = point(px((corner[0] - left) as f32), px((corner[1] - top) as f32));
    s.drag(cx, at, by, Modifiers::none());
    let poses = s.surface.read_with(&s.vcx, |g, _| g.poses());
    let l0 = poses.iter().find(|p| p.id == "l0").unwrap();
    assert_eq!(l0.scale, [200., 200.]);
    assert_eq!(
        l0.position,
        [left + (200. - left) * 2., top + (150. - top) * 2.]
    );
    // A parametric shape grows by its size, not its scale.
    let l1 = layer_value(&s.rig.kernel, "l1").unwrap();
    assert_eq!(channel(&l1, false, "scale.x"), Some(100.));
    let w = l1.get("d").and_then(|d| d.get("w")).cloned().unwrap();
    assert_eq!(
        w.as_f64()
            .or_else(|| w.get("v").and_then(pm_model::Value::as_f64)),
        Some(200.)
    );
    s.undo(cx);
    let poses = s.surface.read_with(&s.vcx, |g, _| g.poses());
    assert!(poses.iter().all(|p| p.scale == [100., 100.]));
}

fn world(kernel: &pm_kernel::KernelHandle, id: &str) -> [f64; 6] {
    let p = kernel.published(PROJECT).unwrap();
    pm_kernel::QueryMirror::new(&p)
        .queries()
        .world_matrix(None, id, 0.5)
        .unwrap()
}
fn select_ids(s: &mut Stage, cx: &mut TestAppContext, ids: &[&str]) {
    let ids = ids.iter().map(|i| (*i).to_owned()).collect();
    let select = SelectionCall::Layers { ids, add: false };
    s.rig
        .kernel
        .call(PROJECT, Request::new(Caller::native(), select))
        .wait()
        .unwrap()
        .unwrap();
    s.refresh(cx);
}
/// A child of a parent scaled 200 % × 100 % turns with the common box: its
/// world matrix becomes the turned one (rotation, scale and skew solved
/// under the parent), not a rotation channel bumped by the angle.
#[gpui::test]
fn the_common_box_turns_a_child_of_a_nonuniform_parent_exactly(cx: &mut TestAppContext) {
    let parented = pair()
        .replacen(
            r#""id":"l0","type":"image","name":"Square","from":0,"dur":2,"on":true,"p":{"#,
            r#""id":"l0","parent":"pa","type":"image","name":"Square","from":0,"dur":2,"on":true,"p":{"skew":{"v":0,"kf":[]},"#,
            1,
        )
        .replace(
            r#""layers":["#,
            r#""layers":[{"id":"pa","type":"null","name":"Parent","from":0,"dur":2,"on":true,"p":{"position.x":{"v":0,"kf":[]},"position.y":{"v":0,"kf":[]},"scale.x":{"v":200,"kf":[]},"scale.y":{"v":100,"kf":[]},"rotation":{"v":0,"kf":[]},"anchor.x":{"v":0,"kf":[]},"anchor.y":{"v":0,"kf":[]},"opacity":{"v":100,"kf":[]}},"d":{},"fx":[]},"#,
        );
    let mut s = unposed(cx, &parented);
    select_ids(&mut s, cx, &["l0", "l1"]);
    let before = world(&s.rig.kernel, "l0");
    let (handle, centre, _) = s
        .surface
        .read_with(&s.vcx, |g, _| g.group_points())
        .unwrap();
    // A quarter turn clockwise about the box centre.
    let r = [handle[0] - centre[0], handle[1] - centre[1]];
    let to = [centre[0] - r[1], centre[1] + r[0]];
    let at = s.point_of(handle);
    s.drag(
        cx,
        at,
        point(
            px((to[0] - handle[0]) as f32),
            px((to[1] - handle[1]) as f32),
        ),
        Modifiers::shift(),
    );
    let after = world(&s.rig.kernel, "l0");
    let w = before;
    let turned = [-w[1], w[0], -w[3], w[2]];
    for (got, want) in after[..4].iter().zip(turned) {
        assert!((got - want).abs() < 1e-3, "{after:?} vs {turned:?}");
    }
}
/// A mirrored layer (negative scale) stays mirrored through a common resize.
#[gpui::test]
fn the_common_box_keeps_a_mirrored_layer_mirrored(cx: &mut TestAppContext) {
    let mirrored = pair().replacen(
        r#""scale.x":{"v":100,"kf":[]}"#,
        r#""scale.x":{"v":-100,"kf":[]}"#,
        1,
    );
    let mut s = unposed(cx, &mirrored);
    select_ids(&mut s, cx, &["l0", "l1"]);
    let (_, centre, corner) = s
        .surface
        .read_with(&s.vcx, |g, _| g.group_points())
        .unwrap();
    let at = s.point_of(corner);
    let by = [(corner[0] - centre[0]) * 2., (corner[1] - centre[1]) * 2.];
    s.drag(
        cx,
        at,
        point(px(by[0] as f32), px(by[1] as f32)),
        Modifiers::none(),
    );
    let l0 = layer_value(&s.rig.kernel, "l0").unwrap();
    assert_eq!(channel(&l0, false, "scale.x"), Some(-200.));
    assert_eq!(channel(&l0, false, "scale.y"), Some(200.));
}
#[test]
fn solving_local_transforms_round_trips_and_keeps_mirrors() {
    use crate::gizmo::solve_local;
    // An unrotated child of a 200 % × 100 % parent, turned 45°.
    let parent = Some([2., 0., 0., 1.]);
    let (s, c) = 45f64.to_radians().sin_cos();
    let desired = [2. * c, 2. * s, -s, c];
    let local = solve_local(parent, desired, 1., 0.).unwrap();
    // Rotating a child of a non-uniform parent needs skew, not just rotation.
    assert!(local.skew.abs() > 1., "{local:?}");
    let mirrored = solve_local(None, [-1., 0., 0., 1.], -1., 0.).unwrap();
    assert_eq!((mirrored.scale_x, mirrored.rotation.round()), (-1., 0.));
    assert!(solve_local(Some([1., 0., 0., 0.]), desired, 1., 0.).is_none());
}
/// The first Pen click with nothing selected makes a path layer, its path
/// and vertex as one undo step; the next click extends that same path.
#[gpui::test]
fn the_first_pen_click_makes_a_path_layer_in_one_undo_step(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let clear = SelectionCall::Layers {
        ids: vec![],
        add: false,
    };
    s.rig
        .kernel
        .call(PROJECT, Request::new(Caller::native(), clear))
        .wait()
        .unwrap()
        .unwrap();
    s.recorder.tool.set(Some(Tool::Pen));
    s.surface.update(&mut s.vcx, |g, cx| g.refresh(cx));
    cx.run_until_parked();
    let count = |s: &Stage| {
        let p = s.rig.kernel.published(PROJECT).unwrap();
        pm_kernel::model::list_layers(p.snapshot.root().unwrap(), None).len()
    };
    let before = count(&s);
    let click = |s: &mut Stage, cx: &mut TestAppContext, at: [f64; 2]| {
        let p = s.point_of(at);
        s.vcx.simulate_click(p, Modifiers::none());
        let surface = s.surface.clone();
        settle(cx, move |cx| surface.read_with(cx, |g, _| g.idle()));
        s.surface.update(&mut s.vcx, |g, cx| g.refresh(cx));
    };
    click(&mut s, cx, [400., 300.]);
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers().len() == 1
    });
    let surface = s.surface.clone();
    settle(cx, move |cx| {
        surface.read_with(cx, |g, _| g.pen_paths() == [1])
    });
    click(&mut s, cx, [500., 300.]);
    let surface = s.surface.clone();
    settle(cx, move |cx| {
        surface.read_with(cx, |g, _| g.pen_paths() == [2])
    });
    assert_eq!(count(&s), before + 1);
    let undo = || Request::new(Caller::native(), Call::History(HistoryCall::Undo));
    s.rig.kernel.call(PROJECT, undo()).wait().unwrap().unwrap();
    s.rig.kernel.call(PROJECT, undo()).wait().unwrap().unwrap();
    assert_eq!(count(&s), before, "two clicks, two steps, nothing left");
}
/// A new path layer's path closes on a click on its first vertex, as a
/// mask path does.
#[gpui::test]
fn a_click_on_the_first_vertex_closes_a_new_path_layer(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let clear = SelectionCall::Layers {
        ids: vec![],
        add: false,
    };
    s.rig
        .kernel
        .call(PROJECT, Request::new(Caller::native(), clear))
        .wait()
        .unwrap()
        .unwrap();
    s.recorder.tool.set(Some(Tool::Pen));
    s.surface.update(&mut s.vcx, |g, cx| g.refresh(cx));
    cx.run_until_parked();
    let click = |s: &mut Stage, cx: &mut TestAppContext, at: [f64; 2]| {
        let p = s.point_of(at);
        s.vcx.simulate_click(p, Modifiers::none());
        let surface = s.surface.clone();
        settle(cx, move |cx| surface.read_with(cx, |g, _| g.idle()));
        s.surface.update(&mut s.vcx, |g, cx| g.refresh(cx));
    };
    for (n, at) in [[300., 100.], [400., 100.], [400., 250.], [300., 250.]]
        .into_iter()
        .enumerate()
    {
        click(&mut s, cx, at);
        let surface = s.surface.clone();
        settle(cx, move |cx| {
            surface.read_with(cx, |g, _| g.pen_paths() == [n + 1])
        });
    }
    click(&mut s, cx, [300., 100.]);
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        let p = kernel.published(PROJECT).unwrap();
        let id = p.selection.layers().first().cloned();
        id.and_then(|id| layer_value(&kernel, &id))
            .is_some_and(|l| closed_somewhere(&l))
    });
}
/// Whether any `closed` channel under `v` is on (a mask's or a shape path's).
fn closed_somewhere(v: &pm_model::Value) -> bool {
    if let Some(map) = v.as_object() {
        let on = map
            .get("closed")
            .and_then(|c| c.get("v").or(Some(c)))
            .is_some_and(|c| *c == pm_model::Value::Bool(true));
        return on || map.values().any(closed_somewhere);
    }
    v.as_array().is_some_and(|a| a.iter().any(closed_somewhere))
}
/// Escape during a marquee leaves the selection as it was.
#[gpui::test]
fn escape_during_a_marquee_keeps_the_selection(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &pair());
    let from = s.point_of([20., 20.]);
    s.vcx
        .simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    let surface = s.surface.clone();
    settle(cx, move |cx| surface.read_with(cx, |g, _| g.idle()));
    s.vcx.simulate_mouse_move(
        from + point(px(60.), px(60.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.run_until_parked();
    assert!(s.surface.update(&mut s.vcx, |g, cx| g.cancel(cx)));
    s.vcx.simulate_mouse_up(
        from + point(px(60.), px(60.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.run_until_parked();
    let p = s.rig.kernel.published(PROJECT).unwrap();
    assert_eq!(p.selection.layers(), ["l0".to_string()]);
}
/// A layer is selected (the kernel's selection, as a click makes it) in a
/// composition the viewer reports with no width, and one whose position is
/// beyond what a float holds. The viewer's gizmos publish their overlay lines for it;
/// what the editing viewer hands the engine from them (guides first, then
/// the gizmo's lines, as `Controller::set_gizmos` does) is drawable, where
/// unwrapping the lines as they came aborted the app on the main thread.
#[gpui::test]
fn a_selection_in_a_zero_size_or_far_off_composition_publishes_drawable_overlays(
    cx: &mut TestAppContext,
) {
    use pm_engine_service::OverlayGeometry;
    let drawable = |l: &OverlayLine| l.ends.iter().chain(&l.color).all(|v| v.is_finite());
    let mut undrawable = 0;
    for (project, size) in [
        // The viewer reports a composition with no width (the size overlay
        // math divides by); the stored project is an ordinary one, because
        // this rig's panel mount refuses a stored width of 0.
        (
            project_of("shape").replace("Square", "No width"),
            [0., 360.],
        ),
        (
            project_of("shape").replace(r#""position.x":{"v":200"#, r#""position.x":{"v":1e42"#),
            [640., 360.],
        ),
    ] {
        assert_ne!(project, project_of("shape"), "the fixture was changed");
        let mut s = unposed(cx, &project);
        s.recorder.size.set(Some(size));
        s.surface.update(&mut s.vcx, |s, cx| s.refresh(cx));
        let surface = s.surface.clone();
        settle(cx, move |cx| surface.read_with(cx, |s, _| s.idle()));
        let lines = s.recorder.lines.borrow().clone();
        undrawable += lines.iter().filter(|l| !drawable(l)).count();
        if lines.iter().any(|l| !drawable(l)) {
            assert!(
                OverlayGeometry::new(lines.clone()).is_err(),
                "taken as they came, these lines are refused"
            );
        }
        let guides = crate::viewer2::overlay_lines(true, true);
        // Never fails and never panics, whatever the lines hold.
        let _published =
            OverlayGeometry::lossy(guides.iter().copied().chain(lines.iter().copied()));
        // What it keeps is exactly what the engine accepts.
        let kept: Vec<OverlayLine> = guides
            .iter()
            .chain(lines.iter().filter(|l| drawable(l)))
            .copied()
            .take(OverlayGeometry::MAX_LINES)
            .collect();
        assert!(kept.len() >= guides.len(), "the guides are kept");
        OverlayGeometry::new(kept).expect("drawable overlay geometry");
    }
    assert!(
        undrawable > 0,
        "the selection really produced lines that cannot be drawn"
    );
}
/// Option-drag in the viewer: the layer stays and a copy takes the move,
/// selected; one Undo step removes it.
#[gpui::test]
fn option_dragging_the_body_leaves_the_layer_and_moves_a_copy(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let layers = |s: &Stage| {
        let p = s.rig.kernel.published(PROJECT).unwrap();
        pm_kernel::model::list_layers(p.snapshot.root().unwrap(), None)
    };
    let position = |s: &Stage, id: &str| {
        let p = s.rig.kernel.published(PROJECT).unwrap();
        ["position.x", "position.y"].map(|channel| {
            pm_kernel::QueryMirror::new(&p)
                .queries()
                .evaluate(None, id, channel, 0.)
                .unwrap()
                .as_f64()
                .unwrap()
        })
    };
    let original = layers(&s)[0].id.clone();
    let before = position(&s, &original);
    let (body, ..) = s.points();
    let at = s.point_of([body[0] + 30., body[1] + 30.]);
    let option = Modifiers {
        alt: true,
        ..Modifiers::none()
    };
    s.drag(cx, at, point(px(40.), px(-10.)), option);
    let after = layers(&s);
    assert_eq!(after.len(), 2, "one copy");
    assert_eq!(position(&s, &original), before, "the layer stays");
    let copy = after.iter().find(|l| l.id != original).unwrap().id.clone();
    assert_eq!(position(&s, &copy), [before[0] + 40., before[1] - 10.]);
    let p = s.rig.kernel.published(PROJECT).unwrap();
    assert_eq!(
        p.selection.layers(),
        std::slice::from_ref(&copy),
        "the copy is selected"
    );
    assert_eq!(p.history.undo_label.as_deref(), Some("Duplicate"));
    drop(p);
    // The copy was the selection, so Undo leaves no box to wait for.
    s.rig
        .kernel
        .call(
            PROJECT,
            Request::new(Caller::native(), Call::History(HistoryCall::Undo)),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(layers(&s).len(), 1, "one Undo step");
    assert_eq!(position(&s, &original), before);
}

/// View ▸ Show Rulers: a press in the top ruler makes a horizontal guide
/// that follows the pointer and settles on the composition's centre within
/// reach; dragged back into the ruler it goes.
#[gpui::test]
fn a_guide_comes_out_of_a_ruler_settles_and_goes_back(cx: &mut TestAppContext) {
    use crate::gizmo::viewer_rulers::{Axis, RulerGuide};
    let mut s = stage(cx);
    s.surface
        .update(&mut s.vcx, |g, cx| g.set_rulers(true, false, cx));
    let origin = bounds(&mut s.vcx, "viewer.gizmos").origin;
    let none = Modifiers::none();
    // The top band is the ruler's height; the frame starts 20 pt in.
    let ruler = origin + point(px(100.), px(8.));
    s.vcx.simulate_mouse_down(ruler, MouseButton::Left, none);
    // 178 composition pixels down is 2 from the centre of a 360 tall frame.
    let near_centre = origin + point(px(100.), px(20. + 178.));
    s.vcx
        .simulate_mouse_move(near_centre, MouseButton::Left, none);
    s.vcx
        .simulate_mouse_up(near_centre, MouseButton::Left, none);
    let guides = s
        .surface
        .read_with(&s.vcx, |g, _| g.ruler_guides().to_vec());
    assert_eq!(
        guides,
        vec![RulerGuide {
            axis: Axis::Horizontal,
            at: 180.
        }]
    );
    assert!(
        s.recorder.navigation.borrow().is_empty(),
        "a guide drag moves no view"
    );
    // Back into the ruler: gone.
    let on_guide = origin + point(px(300.), px(20. + 180.));
    s.vcx.simulate_mouse_down(on_guide, MouseButton::Left, none);
    s.vcx.simulate_mouse_move(ruler, MouseButton::Left, none);
    s.vcx.simulate_mouse_up(ruler, MouseButton::Left, none);
    assert!(
        s.surface
            .read_with(&s.vcx, |g, _| g.ruler_guides().is_empty())
    );
}

/// The middle button pans whatever the tool; Space held over the viewer is
/// the hand, and its release says whether it panned (then it does not play).
#[gpui::test]
fn the_middle_button_and_space_pan_whatever_the_tool(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let none = Modifiers::none();
    let from = s.point_of([10., 10.]);
    let to = from + point(px(30.), px(12.));
    s.vcx.simulate_mouse_down(from, MouseButton::Middle, none);
    s.vcx.simulate_mouse_move(to, MouseButton::Middle, none);
    s.vcx.simulate_mouse_up(to, MouseButton::Middle, none);
    let pans = |s: &Stage| {
        s.recorder
            .navigation
            .borrow()
            .iter()
            .filter(|n| matches!(n, Navigate::Pan(_)))
            .count()
    };
    assert!(pans(&s) >= 1, "the middle button pans");
    let before = pans(&s);
    let surface = s.surface.clone();
    s.vcx
        .update(|_, cx| surface.update(cx, |g, cx| g.arm_space(cx)));
    s.drag(cx, from, point(px(25.), px(0.)), none);
    assert!(
        pans(&s) > before,
        "Space and a press pan, not move the layer"
    );
    let surface = s.surface.clone();
    let panned = s
        .vcx
        .update(|window, cx| surface.update(cx, |g, cx| g.release_space(window, cx)));
    assert!(panned, "it panned: the release does not play");
    let surface = s.surface.clone();
    s.vcx
        .update(|_, cx| surface.update(cx, |g, cx| g.arm_space(cx)));
    let surface = s.surface.clone();
    let panned = s
        .vcx
        .update(|window, cx| surface.update(cx, |g, cx| g.release_space(window, cx)));
    assert!(!panned, "no drag: the release plays");
}
// Viewer selection parity with the shipped viewer (viewer.ts onDown, dblclick).
fn select(s: &Stage, ids: &[&str]) {
    s.rig
        .kernel
        .call(
            PROJECT,
            Request::new(
                Caller::native(),
                SelectionCall::Layers {
                    ids: ids.iter().map(|id| id.to_string()).collect(),
                    add: false,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
}
fn selected(s: &Stage) -> Vec<String> {
    s.rig
        .kernel
        .published(PROJECT)
        .unwrap()
        .selection
        .layers()
        .to_vec()
}
/// Down and up in place, then the surface's picks and queue settle.
fn click(s: &mut Stage, cx: &mut TestAppContext, comp: [f64; 2], m: Modifiers, count: usize) {
    let at = s.point_of(comp);
    s.vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: m,
        click_count: count,
        first_mouse: false,
    });
    s.vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: m,
        click_count: count,
    });
    let surface = s.surface.clone();
    settle(cx, move |cx| surface.read_with(cx, |s, _| s.idle()));
}
fn grouped() -> String {
    pair()
        .replace(
            r#""id":"l1","type":"shape""#,
            r#""id":"l1","group":"g","type":"shape""#,
        )
        .replace(
            r#""layers":[{"#,
            r#""layers":[{"id":"g","type":"group","name":"Group","from":0,"dur":2,"on":true,"p":{"position.x":{"v":0,"kf":[]},"position.y":{"v":0,"kf":[]},"scale.x":{"v":100,"kf":[]},"scale.y":{"v":100,"kf":[]},"rotation":{"v":0,"kf":[]},"opacity":{"v":100,"kf":[]}},"d":{},"fx":[]},{"#,
        )
}
/// A click on one layer of a multiple selection keeps the selection: it was
/// a move that never moved (viewer.ts `startMove(... click)`).
#[gpui::test]
fn a_click_on_a_member_of_a_multiple_selection_keeps_it(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &pair());
    select(&s, &["l0", "l1"]);
    s.refresh(cx);
    click(&mut s, cx, [460., 160.], Modifiers::none(), 1);
    assert_eq!(selected(&s), ["l0", "l1"]);
    // ⇧ on a member still takes it out.
    click(&mut s, cx, [460., 160.], Modifiers::shift(), 1);
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() == ["l0".to_string()]
    });
}
/// The gap inside a multiple selection's common box belongs to the
/// selection: a drag there moves every layer, a click there clears.
#[gpui::test]
fn the_gap_in_a_multiple_selection_moves_it_and_a_click_there_clears(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &pair());
    select(&s, &["l0", "l1"]);
    s.refresh(cx);
    let at = s.point_of([380., 200.]);
    s.drag(cx, at, point(px(0.), px(20.)), Modifiers::none());
    let p = s.rig.kernel.published(PROJECT).unwrap();
    let mut mirror = pm_kernel::QueryMirror::new(&p);
    let mut y = |id: &str| {
        mirror
            .queries()
            .evaluate(None, id, "position.y", 0.5)
            .unwrap()
            .as_f64()
            .unwrap()
    };
    assert_eq!((y("l0"), y("l1")), (170., 170.));
    assert_eq!(selected(&s), ["l0", "l1"]);
    click(&mut s, cx, [380., 200.], Modifiers::none(), 1);
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers().is_empty()
    });
}
/// A click selects the outermost group; a double-click descends one level
/// to the member under the pointer (viewer.ts drillTarget).
#[gpui::test]
fn a_double_click_descends_into_the_group_under_the_pointer(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &grouped());
    select(&s, &[]);
    click(&mut s, cx, [460., 160.], Modifiers::none(), 1);
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() == ["g".to_string()]
    });
    click(&mut s, cx, [460., 160.], Modifiers::none(), 2);
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() == ["l1".to_string()]
    });
    // Nothing deeper: another double-click leaves the member selected.
    click(&mut s, cx, [460., 160.], Modifiers::none(), 2);
    assert_eq!(selected(&s), ["l1"]);
    assert!(s.recorder.opened.borrow().is_empty());
}
/// A double-click on a precomp opens its composition (viewer.ts dblclick).
#[gpui::test]
fn a_double_click_on_a_precomp_opens_its_composition(cx: &mut TestAppContext) {
    let precomp = pair()
        .replace(
            r#""id":"l1","type":"shape""#,
            r#""id":"l1","type":"precomp""#,
        )
        .replace(r#""d":{"w":100,"h":100},"fx":[]}]}"#, r#""d":{"comp":"c2","w":100,"h":100},"fx":[]}]}"#)
        .replace(
            r#""comps":{}"#,
            r##""comps":{"c2":{"id":"c2","name":"Inner","w":100,"h":100,"fps":30,"dur":2,"bg":"#000000","layers":[]}}"##,
        );
    assert!(precomp.contains(r#""comp":"c2""#));
    let mut s = stage_on(cx, &precomp);
    click(&mut s, cx, [460., 160.], Modifiers::none(), 1);
    click(&mut s, cx, [460., 160.], Modifiers::none(), 2);
    let recorder = s.recorder.clone();
    settle(cx, move |_| *recorder.opened.borrow() == ["c2".to_string()]);
}
/// Hovering outlines what a click would select: the outermost group, the
/// leaf with ⌘, nothing over a layer already selected (viewer.ts hover).
#[gpui::test]
fn hovering_outlines_what_a_click_would_select(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &grouped());
    let hover = |s: &mut Stage, cx: &mut TestAppContext, comp: [f64; 2], m: Modifiers| {
        let at = s.point_of(comp);
        s.vcx.simulate_mouse_move(at, None, m);
        let surface = s.surface.clone();
        settle(cx, move |cx| surface.read_with(cx, |s, _| s.idle()));
        s.surface.read_with(&s.vcx, |g, _| g.hovered())
    };
    // l0 is selected by the stage.
    assert_eq!(hover(&mut s, cx, [460., 160.], Modifiers::none()), Some("g".into()));
    assert_eq!(hover(&mut s, cx, [460., 160.], Modifiers::command()), Some("l1".into()));
    assert_eq!(hover(&mut s, cx, [210., 160.], Modifiers::none()), None);
    assert_eq!(hover(&mut s, cx, [600., 340.], Modifiers::none()), None);
    // The outline is drawn: four more lines than the selection alone.
    let before = s.recorder.lines.borrow().len();
    hover(&mut s, cx, [460., 160.], Modifiers::none());
    assert_eq!(s.recorder.lines.borrow().len(), before + 4);
    // A press takes it away.
    let at = s.point_of([460., 160.]);
    s.vcx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    assert_eq!(s.surface.read_with(&s.vcx, |g, _| g.hovered()), None);
    s.vcx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
}
/// A double-click on an imported model picks the part under the pointer:
/// the model alone is selected and Properties is asked to reveal that
/// part's material slot (viewer.ts dblclick → scene3d.pickPart).
#[gpui::test]
fn a_double_click_on_an_imported_model_focuses_the_part_under_the_pointer(
    cx: &mut TestAppContext,
) {
    let mut s = stage(cx);
    let dir = std::env::temp_dir().join(pm_kernel::new_id("pm-part-pick"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("camera.glb");
    // The body covers the middle of the frame; the lens sits to its right.
    std::fs::write(
        &path,
        pm_scene3d::import::quads_glb(&[
            ("Body", [[-2., -1., 0.], [0.3, -1., 0.], [0.3, 1., 0.], [-2., 1., 0.]]),
            ("Lens", [[0.6, -1., 0.], [2., -1., 0.], [2., 1., 0.], [0.6, 1., 0.]]),
        ]),
    )
    .unwrap();
    *s.recorder.assets.borrow_mut() = Some(Arc::new(pm_scene3d::geometry::FileAssets::new(
        [("asset".to_string(), path)].into(),
    )));
    let transact = |s: &Stage, command: pm_kernel::Command| {
        let call = Call::Transact {
            label: None,
            expected_revision: None,
            body: pm_kernel::WriteBody::Commands(vec![command]),
        };
        let ack = s
            .rig
            .kernel
            .call(PROJECT, Request::new(Caller::native(), call))
            .wait()
            .unwrap()
            .unwrap();
        assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)), "{:?}", ack.reply);
    };
    transact(
        &s,
        pm_kernel::MediaCommand::AddAsset {
            asset_id: "asset".into(),
            asset: pm_scene3d::to_model(
                &serde_json::json!({"id":"asset","kind":"model","name":"camera.glb","format":"glb"}),
            ),
        }
        .into(),
    );
    transact(
        &s,
        pm_kernel::Scene3dCommand::Apply {
            comp: None,
            args: pm_scene3d::to_model(&serde_json::json!({"operation":"add_object","id":"model",
                "object":{"source":{"assetId":"asset"},"useSourceMaterials":true}})),
            selection: vec![],
            time: 0.,
            autokey: false,
        }
        .into(),
    );
    s.refresh(cx);
    let surface = s.surface.clone();
    settle(cx, move |cx| surface.read_with(cx, |g, _| g.studio_active()));
    // The first click selects the model, the second picks its part.
    click(&mut s, cx, [320., 180.], Modifiers::none(), 1);
    click(&mut s, cx, [320., 180.], Modifiers::none(), 2);
    let recorder = s.recorder.clone();
    settle(cx, move |_| !recorder.parts.borrow().is_empty());
    assert_eq!(*s.recorder.parts.borrow(), [("model".to_string(), Some(0))]);
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() == ["model".to_string()]
    });
    // Further right along the middle, the lens answers with its own slot.
    for x in (330..640).step_by(10) {
        click(&mut s, cx, [x as f64, 180.], Modifiers::none(), 2);
        if s.recorder.parts.borrow().last().is_some_and(|p| p.1 == Some(1)) {
            break;
        }
    }
    assert_eq!(s.recorder.parts.borrow().last(), Some(&("model".to_string(), Some(1))));
    let _ = std::fs::remove_dir_all(dir);
}
