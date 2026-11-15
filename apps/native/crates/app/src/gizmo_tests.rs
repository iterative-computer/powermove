//! Viewer gizmos: real pointer input through the mounted viewer panel into kernel
//! gestures. The scene is real; the engine overlay is observed at its entry point.
use crate::gizmo::{GizmoSurface, Host, Navigate, Tool, ViewFrame, text::TextInput};
use crate::v4_view::{Natives, SurfaceView};
use crate::v4_view_tests::{
    PROJECT, Rig, bounds, publication, rig, rig_project, settle, tokens, window,
};
use gpui::{
    App, AppContext, Bounds, CursorStyle, Entity, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, Pixels, Point, TestAppContext, VisualTestContext, point, px,
};
use gpui_macos::native_text::TextEvent;
use pm_engine_service::OverlayLine;
use pm_kernel::{Call, Caller, HistoryCall, Request, SelectionCall};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
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
    /// An entity `frame` reads, as the app's host reads its controller.
    lease: RefCell<Option<Entity<u8>>>,
    /// Files dropped on the viewer.
    files: RefCell<Vec<std::path::PathBuf>>,
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
    fn frame(&self, bounds: Bounds<Pixels>, cx: &App) -> Option<ViewFrame> {
        if let Some(lease) = self.0.lease.borrow().as_ref() {
            let _ = lease.read(cx);
        }
        Some(ViewFrame {
            origin: bounds.origin + point(px(20.), px(20.)),
            scale: 1.,
            size: self.0.size.get().unwrap_or([640., 360.]),
        })
    }
    fn tool(&self, _: &App) -> Tool {
        self.0.tool.get().unwrap_or(Tool::Select)
    }
    fn overlay(
        &self,
        lines: Vec<OverlayLine>,
        _: Option<crate::gizmo_layer::Placement>,
        _: &mut App,
    ) {
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
    fn drop_files(&self, paths: Vec<std::path::PathBuf>, _: bool, _: &mut App) {
        self.0.files.borrow_mut().extend(paths);
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
/// A move's outline is at the pointer as each move event lands, before the
/// kernel has published the step and the pose is read back (on a large
/// project that read trailed the pointer by many frames).
#[gpui::test]
fn a_moves_outline_follows_the_pointer_before_the_kernel_answers(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let (body, ..) = s.points();
    let from = s.point_of([body[0] + 30., body[1] + 30.]);
    let first = |s: &Stage| s.recorder.lines.borrow()[0].ends;
    let before = first(&s);
    s.vcx
        .simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    let to = from + point(px(40.), px(-10.));
    s.vcx
        .simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    // No settle: the kernel's step and the read back have not run.
    let moved = first(&s);
    let comp = |n: f32, side: usize| f64::from(n) * [640., 360.][side];
    assert!(
        (comp(moved[0], 0) - comp(before[0], 0) - 40.).abs() < 0.01,
        "{before:?} -> {moved:?}"
    );
    assert!(
        (comp(moved[1], 1) - comp(before[1], 1) + 10.).abs() < 0.01,
        "{before:?} -> {moved:?}"
    );
    s.vcx
        .simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    let surface = s.surface.clone();
    settle(cx, move |cx| surface.read_with(cx, |g, _| g.idle()));
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
/// The cursor says what a press does (viewer.ts updateStageCursor): move
/// inside the selection, rotate on its rotate handle and with the Rotate
/// tool, zoom in with the Zoom tool and zoom out while ⌥ is held.
#[gpui::test]
fn the_viewer_cursor_says_what_a_press_does(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let (center, _, rotate) = s.points();
    let mut at = |s: &mut Stage, comp: [f64; 2], m: Modifiers| {
        let p = s.point_of(comp);
        s.vcx.simulate_mouse_move(p, None, m);
        cx.run_until_parked();
        s.surface.read_with(&s.vcx, |g, _| g.cursor_style())
    };
    assert_eq!(at(&mut s, center, Modifiers::none()), CursorStyle::Move);
    assert_eq!(at(&mut s, rotate, Modifiers::none()), CursorStyle::Rotate);
    assert_eq!(
        at(&mut s, [-15., -15.], Modifiers::none()),
        CursorStyle::Arrow
    );
    s.recorder.tool.set(Some(Tool::Rotate));
    assert_eq!(
        at(&mut s, [-15., -15.], Modifiers::none()),
        CursorStyle::Rotate
    );
    s.recorder.tool.set(Some(Tool::Zoom));
    assert_eq!(at(&mut s, center, Modifiers::none()), CursorStyle::ZoomIn);
    assert_eq!(at(&mut s, center, Modifiers::alt()), CursorStyle::ZoomOut);
    // Letting go of ⌥ without moving flips it back.
    s.vcx.simulate_modifiers_change(Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        s.surface.read_with(&s.vcx, |g, _| g.cursor_style()),
        CursorStyle::ZoomIn
    );
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
    fn overlay(&self, _: Vec<OverlayLine>, _: Option<crate::gizmo_layer::Placement>, cx: &mut App) {
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
#[gpui::test]
fn releasing_space_inside_the_hosts_own_update_does_not_double_lease(cx: &mut TestAppContext) {
    // `viewer.spaceUp` runs inside the controller's update, and the hover
    // that follows reads the controller through `Host::frame`: it must wait
    // until that update is over (two aborts on 2026-11-13).
    let mut s = stage(cx);
    let lease = s.vcx.update(|_, cx| cx.new(|_| 0u8));
    *s.recorder.lease.borrow_mut() = Some(lease.clone());
    let surface = s.surface.clone();
    s.vcx
        .update(|_, cx| surface.update(cx, |g, cx| g.arm_space(cx)));
    let surface = s.surface.clone();
    let panned = s.vcx.update(|window, cx| {
        lease.update(cx, |_, cx| {
            surface.update(cx, |g, cx| g.release_space(window, cx))
        })
    });
    assert!(!panned, "no drag: the release plays");
    s.vcx.run_until_parked();
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
        kernel
            .published(PROJECT)
            .unwrap()
            .selection
            .layers()
            .is_empty()
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
    assert_eq!(
        hover(&mut s, cx, [460., 160.], Modifiers::none()),
        Some("g".into())
    );
    assert_eq!(
        hover(&mut s, cx, [460., 160.], Modifiers::command()),
        Some("l1".into())
    );
    assert_eq!(hover(&mut s, cx, [210., 160.], Modifiers::none()), None);
    assert_eq!(hover(&mut s, cx, [600., 340.], Modifiers::none()), None);
    // The outline is drawn: four more lines than the selection alone.
    let before = s.recorder.lines.borrow().len();
    hover(&mut s, cx, [460., 160.], Modifiers::none());
    assert_eq!(s.recorder.lines.borrow().len(), before + 4);
    // A press takes it away.
    let at = s.point_of([460., 160.]);
    s.vcx
        .simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    assert_eq!(s.surface.read_with(&s.vcx, |g, _| g.hovered()), None);
    s.vcx
        .simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
}
/// A double-click on an imported model picks the part under the pointer:
/// the model alone is selected and Properties is asked to reveal that
/// part's material slot (viewer.ts dblclick → scene3d.pickPart).
#[gpui::test]
fn a_double_click_on_an_imported_model_focuses_the_part_under_the_pointer(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let dir = std::env::temp_dir().join(pm_kernel::new_id("pm-part-pick"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("camera.glb");
    // The body covers the middle of the frame; the lens sits to its right.
    std::fs::write(
        &path,
        pm_scene3d::import::quads_glb(&[
            (
                "Body",
                [[-2., -1., 0.], [0.3, -1., 0.], [0.3, 1., 0.], [-2., 1., 0.]],
            ),
            (
                "Lens",
                [[0.6, -1., 0.], [2., -1., 0.], [2., 1., 0.], [0.6, 1., 0.]],
            ),
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
        assert!(
            !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
            "{:?}",
            ack.reply
        );
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
    settle(cx, move |cx| {
        surface.read_with(cx, |g, _| g.studio_active())
    });
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
        if s.recorder
            .parts
            .borrow()
            .last()
            .is_some_and(|p| p.1 == Some(1))
        {
            break;
        }
    }
    assert_eq!(
        s.recorder.parts.borrow().last(),
        Some(&("model".to_string(), Some(1)))
    );
    let _ = std::fs::remove_dir_all(dir);
}
/// The stage with a camera, a softbox and a box added as Studio 3D layers.
fn studio_stage(cx: &mut TestAppContext) -> Stage {
    let mut s = stage(cx);
    for args in [
        serde_json::json!({"operation":"add_camera","id":"cam","patch":{"p":{"x":0,"y":1,"z":6,"targetX":0,"targetY":0,"targetZ":0}}}),
        serde_json::json!({"operation":"add_object","id":"box","object":{"source":{"primitive":"box"}}}),
        serde_json::json!({"operation":"add_light","id":"key","light":{"type":"area","p":{"x":1.2,"y":1,"z":1.5,"targetX":0.2,"targetY":0.5,"targetZ":0,"width":1,"height":0.6}}}),
    ] {
        let ack = s
            .rig
            .kernel
            .call(
                PROJECT,
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: None,
                        expected_revision: None,
                        body: pm_kernel::WriteBody::Commands(vec![
                            pm_kernel::Scene3dCommand::Apply {
                                comp: None,
                                args: pm_scene3d::to_model(&args),
                                selection: vec![],
                                time: 0.,
                                autokey: false,
                            }
                            .into(),
                        ]),
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        assert!(
            !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
            "{:?}",
            ack.reply
        );
    }
    s.refresh(cx);
    let surface = s.surface.clone();
    settle(cx, move |cx| {
        surface.read_with(cx, |g, _| g.studio_active())
    });
    s
}
/// Refresh for a 3D selection: it has no 2D poses to wait for.
fn refresh_3d(s: &mut Stage, cx: &mut TestAppContext) {
    s.surface.update(&mut s.vcx, |g, cx| g.refresh(cx));
    let surface = s.surface.clone();
    settle(cx, move |cx| surface.read_with(cx, |g, _| g.idle()));
}
/// A pointer drag on 3D handles, as `Stage::drag` without its 2D refresh.
fn drag_3d(s: &mut Stage, cx: &mut TestAppContext, from: Point<Pixels>, by: Point<Pixels>) {
    let m = Modifiers::none();
    s.vcx.simulate_mouse_down(from, MouseButton::Left, m);
    for step in 1..=3 {
        let at = from + point(by.x * (step as f32 / 3.), by.y * (step as f32 / 3.));
        s.vcx.simulate_mouse_move(at, MouseButton::Left, m);
        let g = s.surface.clone();
        settle(cx, move |cx| g.read_with(cx, |g, _| g.idle()));
    }
    s.vcx.simulate_mouse_up(from + by, MouseButton::Left, m);
    let g = s.surface.clone();
    settle(cx, move |cx| g.read_with(cx, |g, _| g.idle()));
    assert!(
        s.recorder.reports.borrow().is_empty(),
        "{:?}",
        s.recorder.reports.borrow()
    );
    refresh_3d(s, cx);
}
fn undo_3d(s: &mut Stage, cx: &mut TestAppContext) {
    s.rig
        .kernel
        .call(
            PROJECT,
            Request::new(Caller::native(), Call::History(HistoryCall::Undo)),
        )
        .wait()
        .unwrap()
        .unwrap();
    refresh_3d(s, cx);
}
/// The document's layer `id`, as stored.
fn stored(s: &Stage, id: &str) -> serde_json::Value {
    let p = s.rig.kernel.published(PROJECT).unwrap();
    let root = pm_scene3d::from_model(&p.snapshot.root().unwrap().to_value()).unwrap();
    fn find(v: &serde_json::Value, id: &str) -> Option<serde_json::Value> {
        match v {
            serde_json::Value::Object(o) => {
                if o.get("id").and_then(|i| i.as_str()) == Some(id) && o.contains_key("p") {
                    return Some(v.clone());
                }
                o.values().find_map(|c| find(c, id))
            }
            serde_json::Value::Array(a) => a.iter().find_map(|c| find(c, id)),
            _ => None,
        }
    }
    find(&root, id).unwrap_or_else(|| panic!("no layer {id} in {root}"))
}
/// Looking around a 3D composition (scroll, the Hand tool) moves only the
/// editor view: the document — the shot camera's position, aim and keys —
/// is untouched, nothing pans the 2D viewer, and View camera returns.
#[gpui::test]
fn navigating_3d_moves_only_the_editor_view_and_view_camera_returns(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    let document = |s: &Stage| {
        pm_scene3d::from_model(
            &s.rig
                .kernel
                .published(PROJECT)
                .unwrap()
                .snapshot
                .root()
                .unwrap()
                .to_value(),
        )
        .unwrap()
    };
    let before = document(&s);
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_free()));
    // In the 3D view (the editor view; camera view moves the camera itself).
    s.surface
        .update(&mut s.vcx, |g, cx| g.studio_setting("look", "free", cx));
    cx.run_until_parked();
    // A trackpad's two-finger scroll orbits.
    let at = s.point_of([320., 180.]);
    s.vcx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(point(px(60.), px(-20.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let orbited = s.surface.read_with(&s.vcx, |g, _| g.studio_free_pose());
    assert!(orbited.is_some(), "the editor view moved");
    assert!(s.surface.read_with(&s.vcx, |g, _| g.studio_free()));
    // ⇧ pans and a wheel dollies; the Hand tool's drag orbits too.
    for (delta, m) in [
        (
            gpui::ScrollDelta::Pixels(point(px(30.), px(10.))),
            Modifiers::shift(),
        ),
        (gpui::ScrollDelta::Lines(point(0., 3.)), Modifiers::none()),
    ] {
        s.vcx.simulate_event(gpui::ScrollWheelEvent {
            position: at,
            delta,
            modifiers: m,
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.run_until_parked();
    }
    s.recorder.tool.set(Some(Tool::Hand));
    let from = s.point_of([100., 100.]);
    s.vcx
        .simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    s.vcx.simulate_mouse_move(
        from + point(px(80.), px(30.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    s.vcx.simulate_mouse_up(
        from + point(px(80.), px(30.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.run_until_parked();
    s.recorder.tool.set(Some(Tool::Select));
    let moved = s
        .surface
        .read_with(&s.vcx, |g, _| g.studio_free_pose())
        .unwrap();
    assert_ne!(Some(moved), orbited, "the Hand tool moved the editor view");
    // Nothing reached the document or the 2D view.
    assert_eq!(document(&s), before, "navigation never edits the shot");
    assert!(s.recorder.navigation.borrow().is_empty());
    // View camera returns to the shot.
    s.surface
        .update(&mut s.vcx, |g, cx| g.studio_setting("look", "camera", cx));
    let surface = s.surface.clone();
    settle(cx, move |cx| {
        surface.update(cx, |g, cx| {
            g.refresh(cx);
            !g.studio_free()
        })
    });
    assert_eq!(document(&s), before);
}
/// View ▸ 2D / 3D (3) switches between the composition as shot and the
/// scene's editor view, and remembers the choice for this composition.
#[gpui::test]
fn two_d_three_d_toggles_the_look_and_remembers_it_per_composition(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_free()));
    let kept = |s: &Stage| {
        let p = s.rig.kernel.published(PROJECT).unwrap();
        let comp = p.snapshot.render_comp();
        p.ui_state
            .viewer
            .get("studioLookByComp")
            .and_then(|m| m.get(&comp))
            .and_then(pm_model::Value::as_str)
            .map(str::to_owned)
    };
    s.surface
        .update(&mut s.vcx, |g, cx| g.studio_setting("look", "toggle", cx));
    let surface = s.surface.clone();
    settle(cx, move |cx| {
        surface.update(cx, |g, cx| {
            g.refresh(cx);
            g.studio_free()
        })
    });
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        let p = kernel.published(PROJECT).unwrap();
        p.ui_state
            .viewer
            .get("studioLook")
            .and_then(pm_model::Value::as_str)
            == Some("free")
    });
    assert_eq!(kept(&s).as_deref(), Some("free"));
    s.surface
        .update(&mut s.vcx, |g, cx| g.studio_setting("look", "toggle", cx));
    let surface = s.surface.clone();
    settle(cx, move |cx| {
        surface.update(cx, |g, cx| {
            g.refresh(cx);
            !g.studio_free()
        })
    });
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        let p = kernel.published(PROJECT).unwrap();
        p.ui_state
            .viewer
            .get("studioLook")
            .and_then(pm_model::Value::as_str)
            == Some("camera")
    });
    assert_eq!(kept(&s).as_deref(), Some("camera"));
}
/// A light's marker selects it and drags it in the view plane (Electron's
/// light markers); its target stays, and Undo restores the position.
#[gpui::test]
fn a_light_marker_selects_and_moves_the_light_and_undo_restores_it(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    let (marker, _) = s
        .surface
        .read_with(&s.vcx, |g, _| g.studio_light_points("key"))
        .expect("the light has a marker in view");
    let before = stored(&s, "key");
    let from = s.point_of(marker);
    drag_3d(&mut s, cx, from, point(px(40.), px(-20.)));
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() == ["key".to_string()]
    });
    let after = stored(&s, "key");
    assert_ne!(
        after["p"]["position.x"], before["p"]["position.x"],
        "{after}"
    );
    assert_eq!(
        after["d"]["data"]["light"]["p"]["targetX"], before["d"]["data"]["light"]["p"]["targetX"],
        "a plain move keeps the light's aim point"
    );
    undo_3d(&mut s, cx);
    assert_eq!(
        stored(&s, "key")["p"]["position.x"],
        before["p"]["position.x"]
    );
}
/// The selected light's ◆ drags its target: the light stays where it is and
/// now aims elsewhere. Undo restores the aim.
#[gpui::test]
fn dragging_a_lights_target_aims_it_and_undo_restores_it(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    let (marker, _) = s
        .surface
        .read_with(&s.vcx, |g, _| g.studio_light_points("key"))
        .unwrap();
    click(&mut s, cx, marker, Modifiers::none(), 1);
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() == ["key".to_string()]
    });
    refresh_3d(&mut s, cx);
    let (_, target) = s
        .surface
        .read_with(&s.vcx, |g, _| g.studio_light_points("key"))
        .unwrap();
    let before = stored(&s, "key");
    let from = s.point_of(target.expect("an aimed light has a target in view"));
    drag_3d(&mut s, cx, from, point(px(0.), px(-30.)));
    let after = stored(&s, "key");
    assert_eq!(after["p"]["position.x"], before["p"]["position.x"]);
    assert_ne!(
        after["d"]["data"]["light"]["p"]["targetY"], before["d"]["data"]["light"]["p"]["targetY"],
        "{after}"
    );
    // The drag nudges the target from where it was, never from the origin.
    let y = |l: &serde_json::Value| {
        let v = &l["d"]["data"]["light"]["p"]["targetY"];
        v.as_f64().or(v["v"].as_f64()).unwrap()
    };
    assert!(
        (y(&after) - y(&before)).abs() < 1.0,
        "{} -> {}",
        y(&before),
        y(&after)
    );
    assert!(y(&before) > 0.4, "the fixture's target is off the origin");
    undo_3d(&mut s, cx);
    assert_eq!(
        stored(&s, "key")["d"]["data"]["light"]["p"]["targetY"],
        before["d"]["data"]["light"]["p"]["targetY"]
    );
}
/// Writes the overlay strokes the viewer would draw for a selected light, at
/// the shot and while looking around, for offscreen review (no screen needed).
/// `PM_PANEL_CAPTURE=… cargo test … dump_light_overlays -- --ignored`
#[gpui::test]
#[ignore = "writes JSON; set PM_PANEL_CAPTURE"]
fn dump_light_overlays(cx: &mut TestAppContext) {
    let dir = std::env::var("PM_PANEL_CAPTURE").unwrap_or_else(|_| "/tmp/pm-shots/panels".into());
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = studio_stage(cx);
    let dump = |s: &mut Stage, cx: &mut TestAppContext, name: &str| {
        s.surface.update(&mut s.vcx, |g, cx| g.redraw(cx));
        cx.run_until_parked();
        let lines: Vec<_> = s
            .recorder
            .lines
            .borrow()
            .iter()
            .map(|l| serde_json::json!({"ends": l.ends, "color": l.color}))
            .collect();
        std::fs::write(
            format!("{dir}/{name}.json"),
            serde_json::to_string(&serde_json::json!({"size":[640,360],"lines":lines})).unwrap(),
        )
        .unwrap();
    };
    let (marker, _) = s
        .surface
        .read_with(&s.vcx, |g, _| g.studio_light_points("key"))
        .unwrap();
    click(&mut s, cx, marker, Modifiers::none(), 1);
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() == ["key".to_string()]
    });
    refresh_3d(&mut s, cx);
    dump(&mut s, cx, "overlay-shot-light-selected");
    in_3d(&mut s, cx);
    let at = s.point_of([320., 180.]);
    s.vcx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(point(px(140.), px(-40.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    s.vcx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Lines(point(0., 4.)),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    refresh_3d(&mut s, cx);
    dump(&mut s, cx, "overlay-editor-view-light-selected");
}
/// The overlay has a fixed budget (`OverlayGeometry::MAX_LINES`): with three
/// lights, one selected, while looking around, everything still fits, so no
/// marker or handle is cut off.
#[gpui::test]
fn lights_handles_and_the_camera_fit_the_overlay_budget(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    for (id, x) in [("fill", -1.5), ("rim", 0.4)] {
        s.rig
            .kernel
            .call(
                PROJECT,
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: None,
                        expected_revision: None,
                        body: pm_kernel::WriteBody::Commands(vec![
                            pm_kernel::Scene3dCommand::Apply {
                                comp: None,
                                args: pm_scene3d::to_model(&serde_json::json!({"operation":"add_light","id":id,
                                    "light":{"type":"spot","p":{"x":x,"y":1.4,"z":1.,"targetX":0,"targetY":0,"targetZ":0}}})),
                                selection: vec![],
                                time: 0.,
                                autokey: false,
                            }
                            .into(),
                        ]),
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
    }
    refresh_3d(&mut s, cx);
    let (marker, _) = s
        .surface
        .read_with(&s.vcx, |g, _| g.studio_light_points("key"))
        .unwrap();
    click(&mut s, cx, marker, Modifiers::none(), 1);
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| {
        kernel.published(PROJECT).unwrap().selection.layers() == ["key".to_string()]
    });
    in_3d(&mut s, cx);
    let at = s.point_of([320., 180.]);
    s.vcx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(point(px(40.), px(-10.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    refresh_3d(&mut s, cx);
    s.surface.update(&mut s.vcx, |g, cx| g.redraw(cx));
    cx.run_until_parked();
    let n = s.recorder.lines.borrow().len();
    assert!(
        n > 40 && n < pm_engine_service::OverlayGeometry::MAX_LINES,
        "{n} lines"
    );
}

/// Files dragged from Finder and released over the viewer are its: the
/// host gets them (to import and place at the playhead), and the drop goes
/// no further (the window's own drop would import them a second time).
#[gpui::test]
fn files_dropped_on_the_viewer_are_imported_there(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    let at = s.point_of([320., 180.]);
    let paths = vec![std::path::PathBuf::from("/media/drop-a.mov")];
    s.vcx.simulate_event(gpui::FileDropEvent::Entered {
        position: at,
        paths: gpui::ExternalPaths(paths.clone().into()),
    });
    s.vcx
        .simulate_event(gpui::FileDropEvent::Pending { position: at });
    s.vcx
        .simulate_event(gpui::FileDropEvent::Submit { position: at });
    s.vcx.run_until_parked();
    assert_eq!(*s.recorder.files.borrow(), paths);
}

/// Fly navigation (Shift+`, Blender's Fly) through the shot camera: confirm
/// stores the flight on the camera as one Undo (the camera and its point of
/// interest move together, so it looks where the flight ended).
#[gpui::test]
fn flying_through_the_camera_stores_the_flight_as_one_undo(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    let before = stored(&s, "cam");
    s.surface.update(&mut s.vcx, |g, cx| g.studio_setting("fly", "", cx));
    assert!(s.surface.read_with(&s.vcx, |g, _| g.studio_flying()));
    s.surface
        .update(&mut s.vcx, |g, cx| g.studio_fly_advance(3., 1., cx));
    s.surface.update(&mut s.vcx, |g, cx| g.studio_fly_end(true, cx));
    let kernel = s.rig.kernel.clone();
    let z0 = before["p"]["position.z"].clone();
    settle(cx, move |_| {
        let p = kernel.published(PROJECT).unwrap();
        let root = pm_scene3d::from_model(&p.snapshot.root().unwrap().to_value()).unwrap();
        root.to_string().contains("\"cam\"") && {
            fn find(v: &serde_json::Value) -> Option<serde_json::Value> {
                match v {
                    serde_json::Value::Object(o) if o.get("id").and_then(|i| i.as_str()) == Some("cam") && o.contains_key("p") => Some(v.clone()),
                    serde_json::Value::Object(o) => o.values().find_map(find),
                    serde_json::Value::Array(a) => a.iter().find_map(find),
                    _ => None,
                }
            }
            find(&root).is_some_and(|c| c["p"]["position.z"] != z0)
        }
    });
    let after = stored(&s, "cam");
    let num = |v: &serde_json::Value| v.as_f64().or(v["v"].as_f64()).unwrap();
    // It flew forward, toward what it looked at (−Z from z = 6).
    assert!(num(&after["p"]["position.z"]) < num(&before["p"]["position.z"]), "{after}");
    // The point of interest moved with it: the camera still looks straight ahead.
    let dz_cam = num(&after["p"]["position.z"]) - num(&before["p"]["position.z"]);
    let t = |l: &serde_json::Value| num(&l["d"]["data"]["camera"]["p"]["targetZ"]);
    assert!((t(&after) - t(&before) - dz_cam).abs() < 1e-6, "{} {}", t(&after), dz_cam);
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_free()), "back to the shot");
    undo_3d(&mut s, cx);
    assert_eq!(stored(&s, "cam")["p"]["position.z"], before["p"]["position.z"]);
}
/// Cancel returns exactly to where the flight began and writes nothing.
#[gpui::test]
fn cancelling_a_flight_restores_the_view_and_writes_nothing(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    let doc = |s: &Stage| s.rig.kernel.published(PROJECT).unwrap().snapshot.root().unwrap().to_value();
    let before = doc(&s);
    s.surface.update(&mut s.vcx, |g, cx| g.studio_setting("fly", "", cx));
    s.surface
        .update(&mut s.vcx, |g, cx| g.studio_fly_advance(4., 1., cx));
    s.surface.update(&mut s.vcx, |g, cx| g.studio_fly_end(false, cx));
    cx.run_until_parked();
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_flying()));
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_free()), "back to the shot");
    assert_eq!(doc(&s), before);
}
/// In the editor view, a kept flight moves only the view.
#[gpui::test]
fn flying_the_editor_view_never_writes_the_project(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    s.surface
        .update(&mut s.vcx, |g, cx| g.studio_setting("look", "free", cx));
    cx.run_until_parked();
    let doc = |s: &Stage| s.rig.kernel.published(PROJECT).unwrap().snapshot.root().unwrap().to_value();
    let before = doc(&s);
    let start = s.surface.read_with(&s.vcx, |g, _| g.studio_free_pose()).unwrap();
    s.surface.update(&mut s.vcx, |g, cx| g.studio_setting("fly", "", cx));
    s.surface
        .update(&mut s.vcx, |g, cx| g.studio_fly_advance(2., 1., cx));
    s.surface.update(&mut s.vcx, |g, cx| g.studio_fly_end(true, cx));
    cx.run_until_parked();
    let end = s.surface.read_with(&s.vcx, |g, _| g.studio_free_pose()).unwrap();
    assert_ne!(end.position, start.position, "the view flew");
    assert!(s.surface.read_with(&s.vcx, |g, _| g.studio_free()));
    assert_eq!(doc(&s), before);
}
/// Shift+` does nothing in a 2D composition.
#[gpui::test]
fn fly_needs_a_3d_composition(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    s.surface.update(&mut s.vcx, |g, cx| g.studio_setting("fly", "", cx));
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_flying()));
}

// ── Blender's default 3D navigation ─────────────────────────────────────
fn in_3d(s: &mut Stage, cx: &mut TestAppContext) {
    s.surface
        .update(&mut s.vcx, |g, cx| g.studio_setting("look", "free", cx));
    cx.run_until_parked();
}
fn scroll(s: &mut Stage, cx: &mut TestAppContext, delta: gpui::ScrollDelta, m: Modifiers) {
    let at = s.point_of([320., 180.]);
    s.vcx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta,
        modifiers: m,
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
}
fn view(s: &Stage) -> pm_scene3d::navigation::Pose {
    s.surface
        .read_with(&s.vcx, |g, _| g.studio_free_pose())
        .expect("an editor view")
}
fn reach(p: pm_scene3d::navigation::Pose) -> f64 {
    pm_scene3d::navigation::length(pm_scene3d::navigation::sub(p.position, p.target))
}
fn applied(s: &Stage) -> usize {
    s.rig.kernel.published(PROJECT).unwrap().history.applied
}
fn ctrl() -> Modifiers {
    Modifiers {
        control: true,
        ..Default::default()
    }
}
/// A mouse wheel notch zooms 1.2× toward the view centre; ⌃+wheel pans
/// sideways and ⇧+wheel up and down, keeping the distance (Blender).
#[gpui::test]
fn the_mouse_wheel_zooms_in_blenders_steps_and_pans_with_ctrl_or_shift(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    in_3d(&mut s, cx);
    let h = applied(&s);
    let a = view(&s);
    scroll(&mut s, cx, gpui::ScrollDelta::Lines(point(0., 1.)), Modifiers::none());
    let b = view(&s);
    assert!((reach(a) / reach(b) - 1.2).abs() < 1e-6, "{} {}", reach(a), reach(b));
    assert_eq!(b.target, a.target, "toward the view centre");
    scroll(&mut s, cx, gpui::ScrollDelta::Lines(point(0., 1.)), ctrl());
    let c = view(&s);
    assert!((reach(c) - reach(b)).abs() < 1e-6 && c.target != b.target);
    assert!((c.target[1] - b.target[1]).abs() < 1e-6, "⌃: sideways only");
    scroll(&mut s, cx, gpui::ScrollDelta::Lines(point(0., 1.)), Modifiers::shift());
    let d = view(&s);
    assert!(d.target[1] != c.target[1], "⇧: up and down");
    // The shot never moved.
    assert_eq!(applied(&s), h);
}
/// A trackpad's two-finger scroll orbits, ⇧ pans, ⌃ zooms; a pinch zooms.
#[gpui::test]
fn the_trackpad_orbits_pans_with_shift_and_zooms_with_ctrl_or_a_pinch(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    in_3d(&mut s, cx);
    let a = view(&s);
    scroll(&mut s, cx, gpui::ScrollDelta::Pixels(point(px(40.), px(0.))), Modifiers::none());
    let b = view(&s);
    assert!(b.target == a.target && (reach(b) - reach(a)).abs() < 1e-6 && b.position != a.position, "orbit");
    scroll(&mut s, cx, gpui::ScrollDelta::Pixels(point(px(20.), px(10.))), Modifiers::shift());
    let c = view(&s);
    assert!(c.target != b.target && (reach(c) - reach(b)).abs() < 1e-6, "pan");
    scroll(&mut s, cx, gpui::ScrollDelta::Pixels(point(px(0.), px(30.))), ctrl());
    let d = view(&s);
    assert!(d.target == c.target && reach(d) < reach(c), "zoom in");
    s.surface.update(&mut s.vcx, |g, cx| {
        g.studio_pinch(0.1, cx);
    });
    assert!(reach(view(&s)) < reach(d), "pinching out zooms in");
}
/// Middle drag orbits, ⇧ pans, ⌃ zooms (Blender's middle mouse).
#[gpui::test]
fn the_middle_button_orbits_pans_and_zooms(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    in_3d(&mut s, cx);
    let mut drag = |s: &mut Stage, m: Modifiers| {
        let from = s.point_of([300., 180.]);
        let to = from + point(px(30.), px(40.));
        s.vcx.simulate_mouse_down(from, MouseButton::Middle, m);
        s.vcx.simulate_mouse_move(to, MouseButton::Middle, m);
        s.vcx.simulate_mouse_up(to, MouseButton::Middle, m);
        cx.run_until_parked();
        view(s)
    };
    let a = view(&s);
    let b = drag(&mut s, Modifiers::none());
    assert!(b.target == a.target && b.position != a.position, "orbit");
    let c = drag(&mut s, Modifiers::shift());
    assert!(c.target != b.target, "pan");
    let d = drag(&mut s, ctrl());
    assert!(d.target == c.target && (reach(d) - reach(c)).abs() > 1e-6, "zoom");
    assert!(s.recorder.navigation.borrow().is_empty(), "the 2D view never panned");
}
/// In camera view every navigation moves the camera itself (Blender's Lock
/// Camera to View, as Fly does): a burst of wheel notches is one undo step,
/// stored once the wheel rests, and the viewer keeps looking through it.
#[gpui::test]
fn navigating_in_camera_view_moves_the_camera_as_one_undo_step(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    let h = applied(&s);
    let before = stored(&s, "cam");
    let num = |v: &serde_json::Value| v.as_f64().or(v["v"].as_f64()).unwrap();
    scroll(&mut s, cx, gpui::ScrollDelta::Lines(point(0., 1.)), Modifiers::none());
    scroll(&mut s, cx, gpui::ScrollDelta::Lines(point(0., 1.)), Modifiers::none());
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_free()), "still the camera's framing");
    cx.executor().advance_clock(std::time::Duration::from_millis(500));
    let kernel = s.rig.kernel.clone();
    settle(cx, move |_| kernel.published(PROJECT).unwrap().history.applied > h);
    assert_eq!(applied(&s), h + 1, "one step");
    let after = stored(&s, "cam");
    // Two notches from z = 6 toward its target at 0: 6 / 1.44.
    let z = num(&after["p"]["position.z"]);
    assert!((z - 6. / 1.44).abs() < 1e-6, "{after}");
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_free()));
    undo_3d(&mut s, cx);
    assert_eq!(stored(&s, "cam")["p"]["position.z"], before["p"]["position.z"]);
}
/// The numpad views (number row too): Front looks down −Z from +Z, orbit
/// steps undo each other, + zooms one notch.
#[gpui::test]
fn numpad_view_keys_set_the_editor_view(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    in_3d(&mut s, cx);
    let h = applied(&s);
    let key = |s: &mut Stage, v: &str| {
        let v = v.to_string();
        s.surface
            .update(&mut s.vcx, move |g, cx| g.studio_setting("nav", &v, cx));
    };
    key(&mut s, "front");
    let f = view(&s);
    let dir = pm_scene3d::navigation::normalize(pm_scene3d::navigation::sub(f.position, f.target));
    assert!((dir[2] - 1.).abs() < 1e-9, "{dir:?}");
    key(&mut s, "orbit-right");
    assert!(view(&s).position != f.position);
    key(&mut s, "orbit-left");
    let back = view(&s);
    assert!((0..3).all(|i| (back.position[i] - f.position[i]).abs() < 1e-9));
    key(&mut s, "zoom-in");
    assert!((reach(f) / reach(view(&s)) - 1.2).abs() < 1e-6);
    assert_eq!(applied(&s), h, "the 3D view writes nothing");
}

#[gpui::test]
fn ortho_toggle_auto_perspective_and_roll_are_editor_only(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    in_3d(&mut s, cx);
    let h = applied(&s);
    let key = |s: &mut Stage, v: &str| {
        s.surface
            .update(&mut s.vcx, |g, cx| g.studio_setting("nav", v, cx));
    };
    let a = view(&s);
    key(&mut s, "persp");
    assert!(view(&s).ortho);
    assert_eq!(view(&s).position, a.position);
    key(&mut s, "persp");
    assert_eq!(view(&s), a);
    key(&mut s, "front");
    assert!(view(&s).ortho);
    key(&mut s, "opposite");
    assert!(view(&s).ortho);
    key(&mut s, "orbit-right");
    assert!(!view(&s).ortho);
    key(&mut s, "roll-left");
    assert_eq!(view(&s).roll, -15.);
    key(&mut s, "roll-right");
    assert_eq!(view(&s).roll, 0.);
    key(&mut s, "front");
    scroll(
        &mut s,
        cx,
        gpui::ScrollDelta::Pixels(point(px(20.), px(0.))),
        Modifiers::none(),
    );
    assert!(!view(&s).ortho);
    assert_eq!(applied(&s), h);
}

#[gpui::test]
fn ortho_and_roll_from_camera_view_leave_the_shot_untouched(cx: &mut TestAppContext) {
    for value in ["persp", "front", "roll-left", "roll-right"] {
        let mut s = studio_stage(cx);
        let h = applied(&s);
        let before = stored(&s, "cam");
        s.surface
            .update(&mut s.vcx, |g, cx| g.studio_setting("nav", value, cx));
        assert!(s.surface.read_with(&s.vcx, |g, _| g.studio_free()));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        assert_eq!(applied(&s), h);
        assert_eq!(stored(&s, "cam"), before);
    }
}

#[gpui::test]
fn two_d_three_d_returns_to_camera_after_the_last_layer_is_removed(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    s.surface.update(&mut s.vcx, |g, cx| g.studio_setting("look", "toggle", cx));
    for _ in 0..3 { undo_3d(&mut s, cx); }
    let surface = s.surface.clone();
    settle(cx, move |cx| surface.update(cx, |g, cx| {
        g.refresh(cx);
        !g.studio_active() && !g.studio_free()
    }));
    s.surface.update(&mut s.vcx, |g, cx| {
        g.studio_setting("look", "toggle", cx);
        assert!(!g.studio_free());
    });
}

#[gpui::test]
fn switching_to_2d_ends_fly_and_releases_its_shortcut_capture(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    s.surface.update(&mut s.vcx, |g, cx| g.studio_setting("fly", "", cx));
    assert!(s.surface.read_with(&s.vcx, |g, _| g.studio_flying()));
    s.surface.update(&mut s.vcx, |g, cx| g.studio_setting("look", "camera", cx));
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_flying()));
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_free()));
}

#[gpui::test]
fn toggling_to_2d_during_fly_uses_the_view_on_screen(cx: &mut TestAppContext) {
    let mut s = studio_stage(cx);
    s.surface.update(&mut s.vcx, |g, cx| g.studio_setting("fly", "", cx));
    s.surface.update(&mut s.vcx, |g, cx| g.studio_setting("look", "toggle", cx));
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_flying()));
    assert!(!s.surface.read_with(&s.vcx, |g, _| g.studio_free()));
}

fn gradient_handle_project(animated: bool) -> String {
    let mut p: serde_json::Value = serde_json::from_str(&project()).unwrap();
    let keys = if animated {
        serde_json::json!([{"i":"start","t":0,"v":20}])
    } else {
        serde_json::json!([])
    };
    p["layers"][0]["fx"] = serde_json::json!([{"id":"ramp","type":"gradient","version":2,"open":true,"on":true,"p":{
        "startX":{"v":20,"kf":keys},"startY":{"v":20,"kf":[]},"endX":{"v":80,"kf":[]},"endY":{"v":80,"kf":[]},"rampShape":{"v":0,"kf":[]}
    }}]);
    p.to_string()
}
#[gpui::test]
fn gradient_handle_drag_is_one_undo_and_keys_animated_params(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &gradient_handle_project(true));
    let before = s
        .rig
        .kernel
        .published(PROJECT)
        .unwrap()
        .snapshot
        .root()
        .unwrap()
        .to_value();
    let points = s.surface.read_with(&s.vcx, |s, _| s.effect_points());
    assert_eq!(points.len(), 2);
    let at = s.point_of(points[0]);
    s.drag(cx, at, point(px(15.), px(8.)), Modifiers::none());
    let p = s.rig.kernel.published(PROJECT).unwrap();
    let mut q = pm_kernel::QueryMirror::new(&p);
    assert_eq!(
        q.queries()
            .evaluate(None, "l0", "ramp.startX", 0.5)
            .unwrap()
            .as_f64(),
        Some(35.)
    );
    let root = p.snapshot.root().unwrap().to_value();
    let keys = root.get("layers").unwrap().as_array().unwrap()[0]
        .get("fx")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .get("p")
        .unwrap()
        .get("startX")
        .unwrap()
        .get("kf")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(keys.len(), 2);
    s.undo(cx);
    let after = s
        .rig
        .kernel
        .published(PROJECT)
        .unwrap()
        .snapshot
        .root()
        .unwrap()
        .to_value();
    assert_eq!(before.get("layers"), after.get("layers"));
}
#[gpui::test]
fn agent_shader_handle_drives_its_uniform(cx: &mut TestAppContext) {
    let mut project: serde_json::Value = serde_json::from_str(&project()).unwrap();
    project["layers"][0]["type"] = serde_json::json!("shader");
    project["layers"][0]["d"] = serde_json::json!({"w":100,"h":100,"code":"uniform float uX; // @param 20\nuniform float uY; // @param 20\nvoid main(){fragColor=vec4(1.);}","uniforms":{"uX":{"v":20,"kf":[]},"uY":{"v":20,"kf":[]}},"viewerHandles":[{"kind":"point","point":{"x":"uX","y":"uY"}}]});
    let mut s = stage_on(cx, &project.to_string());
    let points = s.surface.read_with(&s.vcx, |s, _| s.effect_points());
    assert_eq!(points.len(), 1);
    let at = s.point_of(points[0]);
    s.drag(cx, at, point(px(12.), px(9.)), Modifiers::none());
    let p = s.rig.kernel.published(PROJECT).unwrap();
    let mut q = pm_kernel::QueryMirror::new(&p);
    assert_eq!(
        q.queries()
            .evaluate(None, "l0", "u.uX", 0.5)
            .unwrap()
            .as_f64(),
        Some(32.)
    );
    s.undo(cx);
    let p = s.rig.kernel.published(PROJECT).unwrap();
    let mut q = pm_kernel::QueryMirror::new(&p);
    assert_eq!(
        q.queries()
            .evaluate(None, "l0", "u.uX", 0.5)
            .unwrap()
            .as_f64(),
        Some(20.)
    );
}

#[gpui::test]
fn gradient_handle_drag_inverts_layer_rotation_and_nonuniform_scale(cx: &mut TestAppContext) {
    let mut project: serde_json::Value =
        serde_json::from_str(&gradient_handle_project(false)).unwrap();
    project["layers"][0]["p"]["scale.x"]["v"] = serde_json::json!(200);
    project["layers"][0]["p"]["scale.y"]["v"] = serde_json::json!(50);
    project["layers"][0]["p"]["rotation"]["v"] = serde_json::json!(90);
    let mut s = stage_on(cx, &project.to_string());
    let points = s.surface.read_with(&s.vcx, |s, _| s.effect_points());
    let at = s.point_of(points[0]);
    s.drag(cx, at, point(px(-5.), px(20.)), Modifiers::none());
    let p = s.rig.kernel.published(PROJECT).unwrap();
    let mut q = pm_kernel::QueryMirror::new(&p);
    assert!(
        (q.queries()
            .evaluate(None, "l0", "ramp.startX", 0.5)
            .unwrap()
            .as_f64()
            .unwrap()
            - 30.)
            .abs()
            < 1e-6
    );
    assert!(
        (q.queries()
            .evaluate(None, "l0", "ramp.startY", 0.5)
            .unwrap()
            .as_f64()
            .unwrap()
            - 30.)
            .abs()
            < 1e-6
    );
}

fn gradient_stop_project(animated: bool) -> String {
    let mut p: serde_json::Value = serde_json::from_str(&gradient_handle_project(false)).unwrap();
    p["layers"][0]["fx"][0]["version"] = serde_json::json!(3);
    let stops =
        serde_json::json!([{"position":0,"color":[1,0,0,1]},{"position":1,"color":[0,0,1,1]}]);
    p["layers"][0]["fx"][0]["p"]["stops"] = serde_json::json!({"v":stops,"kf":if animated {serde_json::json!([{"i":"stops0","t":0,"v":stops}])}else{serde_json::json!([])}});
    p.to_string()
}
fn ramp_at(s: &Stage, t: f64) -> [f64; 2] {
    let points = s.surface.read_with(&s.vcx, |g, _| g.effect_points());
    [
        points[0][0] + (points[1][0] - points[0][0]) * t,
        points[0][1] + (points[1][1] - points[0][1]) * t,
    ]
}
fn stop_values(s: &Stage) -> Vec<pm_fx_defs::handles::Stop> {
    s.surface.read_with(&s.vcx, |g, _| g.effect_stop_values())
}
fn add_middle_stop(s: &mut Stage, cx: &mut TestAppContext) {
    let at = ramp_at(s, 0.5);
    click(s, cx, at, Modifiers::none(), 1);
    s.refresh(cx);
}
#[gpui::test]
fn gradient_stop_click_line_adds_interpolated_colour_and_one_undo(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &gradient_stop_project(false));
    add_middle_stop(&mut s, cx);
    let stops = stop_values(&s);
    assert_eq!(stops.len(), 3);
    assert_eq!(stops[1].position, 0.5);
    assert_eq!(stops[1].color, [0.5, 0., 0.5, 1.]);
    s.undo(cx);
    assert_eq!(stop_values(&s).len(), 2);
}
#[gpui::test]
fn gradient_stop_drag_moves_along_line_and_is_one_keyframed_undo(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &gradient_stop_project(true));
    add_middle_stop(&mut s, cx);
    let from = ramp_at(&s, 0.5);
    let to = ramp_at(&s, 0.75);
    let at = s.point_of(from);
    s.drag(
        cx,
        at,
        point(px((to[0] - from[0]) as f32), px((to[1] - from[1]) as f32)),
        Modifiers::none(),
    );
    assert!((stop_values(&s)[1].position - 0.75).abs() < 1e-6);
    let p = s.rig.kernel.published(PROJECT).unwrap();
    let root = p.snapshot.root().unwrap().to_value();
    let keys = root.get("layers").unwrap().as_array().unwrap()[0]
        .get("fx")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .get("p")
        .unwrap()
        .get("stops")
        .unwrap()
        .get("kf")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(keys.len(), 2);
    s.undo(cx);
    assert_eq!(stop_values(&s)[1].position, 0.5);
}
#[gpui::test]
fn gradient_stop_off_line_removal_uses_screen_points_and_preserves_two(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &gradient_stop_project(false));
    add_middle_stop(&mut s, cx);
    let at = s.point_of(ramp_at(&s, 0.5));
    // The diagonal ramp's perpendicular has unit direction (-sqrt(.5),sqrt(.5)).
    let n = (0.5f32).sqrt();
    s.drag(cx, at, point(px(-25. * n), px(25. * n)), Modifiers::none());
    assert_eq!(stop_values(&s).len(), 2);
    s.undo(cx);
    assert_eq!(stop_values(&s).len(), 3);
    let at = s.point_of(ramp_at(&s, 0.5));
    s.drag(cx, at, point(px(-23. * n), px(23. * n)), Modifiers::none());
    assert_eq!(stop_values(&s).len(), 3);
    let at = s.point_of(ramp_at(&s, 0.5));
    s.drag(cx, at, point(px(-25. * n), px(25. * n)), Modifiers::none());
    let endpoint = ramp_at(&s, 0.);
    click(
        &mut s,
        cx,
        [endpoint[0] + 6., endpoint[1]],
        Modifiers::none(),
        1,
    );
    s.vcx.simulate_keystrokes("backspace");
    cx.run_until_parked();
    s.refresh(cx);
    assert_eq!(stop_values(&s).len(), 2);
}
#[gpui::test]
fn gradient_stop_delete_key_removes_selected_stop_and_undo_restores_it(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &gradient_stop_project(false));
    add_middle_stop(&mut s, cx);
    s.vcx.simulate_keystrokes("backspace");
    let g = s.surface.clone();
    settle(cx, move |cx| g.read_with(cx, |g, _| g.idle()));
    s.refresh(cx);
    assert_eq!(stop_values(&s).len(), 2);
    s.undo(cx);
    assert_eq!(stop_values(&s).len(), 3);
}
#[gpui::test]
fn gradient_stop_double_click_opens_anchored_picker_and_edits_rgba(cx: &mut TestAppContext) {
    let mut s = stage_on(cx, &gradient_stop_project(false));
    add_middle_stop(&mut s, cx);
    let at = ramp_at(&s, 0.5);
    click(&mut s, cx, at, Modifiers::none(), 2);
    let b = s
        .surface
        .read_with(&s.vcx, |g, cx| g.effect_picker_rect(cx))
        .expect("anchored picker");
    let anchor = s.point_of(at);
    assert!(f32::from(b.origin.y - anchor.y).abs() < 170.);
    // Drag the R slider from 50% to 100%.
    let from = b.origin + point(px(100.), px(42.));
    s.drag(cx, from, point(px(72.), px(0.)), Modifiers::none());
    assert_eq!(stop_values(&s)[1].color, [1., 0., 0.5, 1.]);
    // Alpha is a separate gesture and undo item.
    let from=b.origin+point(px(172.),px(114.));
    s.drag(cx,from,point(px(-108.),px(0.)),Modifiers::none());
    assert_eq!(stop_values(&s)[1].color,[1.,0.,0.5,0.25]);
    s.undo(cx);assert_eq!(stop_values(&s)[1].color,[1.,0.,0.5,1.]);
    s.undo(cx);
    assert_eq!(stop_values(&s)[1].color, [0.5, 0., 0.5, 1.]);
    s.vcx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        s.surface
            .read_with(&s.vcx, |g, cx| g.effect_picker_rect(cx))
            .is_none()
    );
}

#[gpui::test]
fn agent_registered_shader_effect_stop_handle_drives_its_collection(cx: &mut TestAppContext) {
    let mut p: serde_json::Value = serde_json::from_str(&gradient_stop_project(false)).unwrap();
    p["layers"][0]["fx"][0]["type"] = serde_json::json!("agent.ramp");
    p["layers"][0]["fx"][0]["version"] = serde_json::json!(1);
    let mut s = stage_on(cx, &p.to_string());
    let mut definition = pm_fx_defs::catalog::main_catalog()
        .current("gradient")
        .unwrap()
        .clone();
    definition.id = "agent.ramp".into();
    definition.label = "Agent ramp".into();
    definition.frag = "o=pm_stops(v_uv.x);".into();
    definition.metadata.remove("legacy");
    definition.metadata.remove("rawShader");
    definition
        .metadata
        .insert("version".into(), serde_json::json!(1));
    s.rig
        .kernel
        .state_transaction(pm_kernel::state::StateTransaction {
            ops: vec![pm_kernel::state::StateOp::Register {
                registry: "effects".into(),
                entry: pm_kernel::state::Registration {
                    token: "test.agent.ramp".into(),
                    owner: Caller::native().writer,
                    id: "agent.ramp".into(),
                    definition: pm_model::json::parse(&serde_json::to_string(&definition).unwrap())
                        .unwrap(),
                },
            }],
            ..Default::default()
        })
        .wait()
        .unwrap()
        .unwrap();
    s.refresh(cx);
    add_middle_stop(&mut s, cx);
    assert_eq!(stop_values(&s).len(), 3);
    let from = ramp_at(&s, 0.5);
    let to = ramp_at(&s, 0.75);
    let at = s.point_of(from);
    s.drag(
        cx,
        at,
        point(px((to[0] - from[0]) as f32), px((to[1] - from[1]) as f32)),
        Modifiers::none(),
    );
    assert!((stop_values(&s)[1].position - 0.75).abs() < 1e-6);
    s.undo(cx);
    assert_eq!(stop_values(&s)[1].position, 0.5);
}
