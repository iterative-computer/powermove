use crate::{
    NumField,
    field::{FieldState, drag_listeners, field_api, measure},
    logic::{self, EditEvent},
};
use gpui::{prelude::*, *};
pub struct PointField {
    pub state: FieldState<[f64; 2]>,
    pub min: [f64; 2],
    pub max: [f64; 2],
    pub steps: [Option<f64>; 2],
    axes: [Entity<NumField>; 2],
    bounds: Bounds<Pixels>,
    dragging: bool,
}
impl PointField {
    pub fn new(value: [f64; 2], window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = FieldState::new(value, "Position", cx);
        let axes = std::array::from_fn(|axis| {
            let weak = cx.weak_entity();
            let begin = weak.clone();
            cx.new(|cx| {
                NumField::new(value[axis], window, cx)
                    .label(if axis == 0 { "X" } else { "Y" })
                    .on_begin(move |cx| {
                        begin
                            .update(cx, |this: &mut Self, cx| this.state.begin(cx))
                            .unwrap_or(false)
                    })
                    .on_edit(move |event, cx| {
                        let _ = weak.update(cx, |this: &mut Self, cx| {
                            match event {
                                EditEvent::Begin => {
                                    this.state.begin(cx);
                                }
                                EditEvent::Change(v) => {
                                    let mut p = this.state.value;
                                    p[axis] = *v;
                                    this.state.change(p, cx);
                                }
                                EditEvent::Commit(_) => this.state.commit(cx),
                                EditEvent::Cancel(_) => this.state.cancel(cx),
                            }
                            cx.notify();
                        });
                    })
            })
        });
        cx.on_release(|this, cx| this.state.cancel(cx)).detach();
        cx.observe_window_activation(window, |this, w, cx| {
            if !w.is_window_active() {
                this.dragging = false;
                this.state.cancel(cx);
                cx.notify();
            }
        })
        .detach();
        Self {
            state,
            min: [-100.; 2],
            max: [100.; 2],
            steps: [None; 2],
            axes,
            bounds: Bounds::default(),
            dragging: false,
        }
    }
    field_api!([f64; 2]);
    pub fn range(mut self, min: [f64; 2], max: [f64; 2]) -> Self {
        self.min = min;
        self.max = max;
        self
    }
    pub fn steps(mut self, steps: [Option<f64>; 2]) -> Self {
        self.steps = steps;
        self
    }
    fn snapped(&self, point: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|i| {
            logic::snap(
                point[i],
                self.min[i],
                self.max[i],
                logic::point_step(self.steps[i], self.min[i], self.max[i]),
            )
        })
    }
    fn at(&mut self, p: Point<Pixels>, cx: &mut Context<Self>) {
        let frac = [
            f64::from((p.x - self.bounds.left()) / self.bounds.size.width),
            f64::from((p.y - self.bounds.top()) / self.bounds.size.height),
        ];
        let point = std::array::from_fn(|i| self.min[i] + frac[i] * (self.max[i] - self.min[i]));
        let point = self.snapped(point);
        self.state.change(point, cx);
        cx.notify();
    }
    fn up(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.dragging {
            self.dragging = false;
            self.state.commit(cx);
            cx.notify();
        }
    }
    fn key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = e.keystroke.key.as_str();
        if key == "escape" {
            self.dragging = false;
            self.state.cancel(cx);
        } else {
            let mut p = self.state.current(cx);
            let m = if e.keystroke.modifiers.shift { 10. } else { 1. };
            match key {
                "home" => p = [0.; 2],
                "left" => p[0] -= logic::point_step(self.steps[0], self.min[0], self.max[0]) * m,
                "right" => p[0] += logic::point_step(self.steps[0], self.min[0], self.max[0]) * m,
                "up" => p[1] -= logic::point_step(self.steps[1], self.min[1], self.max[1]) * m,
                "down" => p[1] += logic::point_step(self.steps[1], self.min[1], self.max[1]) * m,
                _ => return,
            }
            self.state.once(self.snapped(p), cx);
        }
        cx.stop_propagation();
        cx.notify();
    }
}
impl Focusable for PointField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.state.focus.clone()
    }
}
impl Render for PointField {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.state.theme(w);
        let value = if self.state.gesture.active() {
            self.state.value
        } else {
            self.state.current(cx)
        };
        for i in 0..2 {
            self.axes[i].update(cx, |field, cx| {
                field.state.appearance = self.state.appearance;
                field.state.size = self.state.size;
                field.state.disabled = self.state.disabled;
                field.state.mixed = self.state.mixed;
                field.options.min = self.min[i];
                field.options.max = self.max[i];
                field.options.step = logic::point_step(self.steps[i], self.min[i], self.max[i]);
                field.set_value(value[i], cx);
            });
        }
        let x = ((value[0] - self.min[0]) / (self.max[0] - self.min[0])).clamp(0., 1.) as f32;
        let y = ((value[1] - self.min[1]) / (self.max[1] - self.min[1])).clamp(0., 1.) as f32;
        div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(
                self.state
                    .well(w)
                    .id("pad")
                    .role(Role::Slider)
                    .aria_value(format!("{}, {}", value[0], value[1]))
                    .relative()
                    .flex_1()
                    .h(px(self.state.size.height() * 2. + 4.))
                    .cursor(CursorStyle::Crosshair)
                    .on_key_down(cx.listener(Self::key))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, e: &MouseDownEvent, w, cx| {
                            if this.state.begin(cx) {
                                this.dragging = true;
                                this.state.focus.focus(w, cx);
                                this.at(e.position, cx);
                            }
                            cx.stop_propagation();
                        }),
                    )
                    .child(measure(cx.entity(), |this, b| this.bounds = b))
                    .child(
                        div()
                            .absolute()
                            .left(relative(0.5))
                            .top_0()
                            .bottom_0()
                            .w(px(1.))
                            .bg(t.ink_3),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(relative(0.5))
                            .left_0()
                            .right_0()
                            .h(px(1.))
                            .bg(t.ink_3),
                    )
                    .when(!self.state.mixed, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(relative(x))
                                .top(relative(y))
                                .ml(px(-4.))
                                .mt(px(-4.))
                                .size(px(8.))
                                .rounded_full()
                                .bg(t.text),
                        )
                    })
                    .when(self.dragging, |d| {
                        d.child(drag_listeners(
                            cx.entity(),
                            |this, e, _, cx| {
                                if this.dragging {
                                    this.at(e.position, cx);
                                }
                            },
                            Self::up,
                        ))
                    }),
            )
            .child(
                div()
                    .w(px(90.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(self.axes.iter().cloned()),
            )
    }
}
