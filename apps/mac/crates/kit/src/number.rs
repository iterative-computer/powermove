use crate::{
    arith,
    field::{FieldState, field_api, measure},
    input::{InputEvent, TextInput},
    logic::{self, NumberOptions, WheelScrub},
};
use gpui::{prelude::*, *};
use std::time::{Duration, Instant};
#[derive(Clone, Copy)]
struct Drag {
    origin: Point<Pixels>,
    start: f64,
    moved: bool,
    visual: bool,
    travel: (f64, f64),
}
pub struct NumField {
    pub state: FieldState<f64>,
    pub options: NumberOptions,
    slider: bool,
    angle: bool,
    editing: bool,
    initial: String,
    input: Entity<TextInput>,
    drag: Option<Drag>,
    bounds: Bounds<Pixels>,
    wheel: WheelScrub,
    wheel_epoch: Instant,
    wheel_task: Option<Task<()>>,
    infinite: Option<crate::native::InfiniteScrub>,
    last_haptic: Option<Instant>,
}
pub type SliderField = NumField;
impl NumField {
    pub fn new(value: f64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = FieldState::new(value, "Value", cx);
        let input = cx.new(|cx| TextInput::new("", "Value", window, cx));
        cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::Commit => this.finish_text(true, cx),
            InputEvent::Cancel => this.finish_text(false, cx),
            InputEvent::Change(_) => {
                cx.notify();
                let _ = window;
            }
        })
        .detach();
        cx.on_focus(&state.focus, window, |this, window, cx| {
            if this.drag.is_none() && !this.state.disabled && !this.slider {
                this.open_editor(window, cx)
            }
        })
        .detach();
        cx.on_release(|this, cx| {
            this.infinite = None;
            this.state.cancel(cx)
        })
        .detach();
        cx.on_blur(&state.focus, window, |this, _, cx| {
            if this.drag.is_some() || this.wheel.active {
                this.drag = None;
                this.infinite = None;
                this.wheel_task = None;
                this.wheel.finish();
                this.state.cancel(cx);
                cx.notify();
            }
        })
        .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.drag = None;
                this.infinite = None;
                this.wheel_task = None;
                this.wheel.finish();
                this.state.cancel(cx);
                cx.notify();
            }
        })
        .detach();
        Self {
            state,
            options: Default::default(),
            slider: false,
            angle: false,
            editing: false,
            initial: String::new(),
            input,
            drag: None,
            bounds: Bounds::default(),
            wheel: Default::default(),
            wheel_epoch: Instant::now(),
            wheel_task: None,
            infinite: None,
            last_haptic: None,
        }
    }
    pub fn slider(
        value: f64,
        min: f64,
        max: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut n = Self::new(value, window, cx);
        n.slider = true;
        n.options.min = min;
        n.options.max = max;
        n
    }
    field_api!(f64);
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.options.min = min;
        self.options.max = max;
        self
    }
    pub fn step(mut self, step: f64) -> Self {
        self.options.step = step;
        self
    }
    pub fn speed(mut self, speed: f64) -> Self {
        self.options.speed = speed;
        self
    }
    pub fn precision(mut self, precision: usize) -> Self {
        self.options.precision = Some(precision);
        self
    }
    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.options.unit = unit.into();
        self
    }
    pub fn angle(mut self, angle: bool) -> Self {
        self.angle = angle;
        self
    }
    fn open_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled || self.editing {
            return;
        }
        self.state.value = self.state.current(cx);
        self.initial = logic::round(self.state.value, 3).to_string();
        self.editing = true;
        self.input.update(cx, |input, cx| {
            input.set_value(self.initial.clone(), cx);
            input.label = self.state.label.clone();
            input.focus(true, window, cx);
        });
        cx.notify();
    }
    fn finish_text(&mut self, commit: bool, cx: &mut Context<Self>) {
        if !self.editing {
            return;
        }
        self.editing = false;
        let raw = self.input.read(cx).value().trim().to_string();
        if commit && raw != self.initial {
            if let Ok(n) = arith::draft(&raw, self.state.current(cx)) {
                let n = logic::round(n, 4);
                if n.is_finite() {
                    self.state.once(n, cx);
                }
            }
        }
        cx.notify();
    }
    fn visual_value(&self, p: Point<Pixels>, cx: &App) -> f64 {
        let min = self.options.min;
        let max = self.options.max;
        let step = if self.options.step > 0. {
            self.options.step
        } else {
            1.
        };
        let n = if self.angle && max - min >= 360. {
            let center = self.bounds.center();
            logic::dial_value(
                f64::from(p.y - center.y)
                    .atan2(f64::from(p.x - center.x))
                    .to_degrees(),
                self.state.current(cx),
                min,
                max,
            )
        } else {
            min + f64::from((p.x - self.bounds.left()) / self.bounds.size.width).clamp(0., 1.)
                * (max - min)
        };
        logic::snap(n, min, max, step)
    }
    fn down(
        &mut self,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
        visual: bool,
    ) {
        if self.state.disabled || self.editing {
            return;
        }
        if self.wheel.active {
            self.state.commit(cx);
            self.wheel.finish();
            self.wheel_task = None;
        }
        if !self.state.begin(cx) {
            return;
        }
        let start = self.state.value;
        self.drag = Some(Drag {
            origin: e.position,
            start,
            moved: visual,
            visual,
            travel: (0., 0.),
        });
        self.state.focus.focus(window, cx);
        if visual {
            let n = self.visual_value(e.position, cx);
            self.state.change(n, cx);
        } else if e.click_count >= 2 {
            self.drag = None;
            self.state.cancel(cx);
            self.open_editor(window, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }
    fn move_drag(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(mut d) = self.drag else {
            return;
        };
        let (dx, dy) = if let Some(infinite) = &self.infinite {
            let Some((x, y)) = infinite.delta() else {
                self.infinite = None;
                self.drag = None;
                self.state.cancel(cx);
                cx.notify();
                return;
            };
            (d.travel.0 + x, d.travel.1 + y)
        } else {
            (
                f64::from(e.position.x - d.origin.x),
                f64::from(e.position.y - d.origin.y),
            )
        };
        if !d.moved && !logic::past_threshold(dx, dy) {
            return;
        }
        if !d.moved && !d.visual {
            self.infinite = crate::native::InfiniteScrub::begin();
        }
        d.moved = true;
        d.travel = (dx, dy);
        self.drag = Some(d);
        let n = if d.visual {
            self.visual_value(e.position, cx)
        } else {
            self.options
                .scrub(d.start, dx, dy, e.modifiers.shift, e.modifiers.alt)
        };
        let changed = self.options.format(n) != self.options.format(self.state.value);
        if changed
            && self
                .last_haptic
                .is_none_or(|t| t.elapsed() >= Duration::from_millis(40))
        {
            crate::native::alignment();
            self.last_haptic = Some(Instant::now());
        }
        self.state.change(n, cx);
        cx.notify();
    }
    fn up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.infinite = None;
        if let Some(d) = self.drag.take() {
            if d.moved {
                self.state.commit(cx);
            } else {
                self.state.cancel(cx);
                self.open_editor(window, cx);
            }
            cx.notify();
        }
    }
    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            return;
        }
        match e.keystroke.key.as_str() {
            "escape" => {
                self.drag = None;
                self.infinite = None;
                self.wheel.finish();
                self.wheel_task = None;
                self.state.cancel(cx);
                self.finish_text(false, cx);
            }
            "enter" | "f2" => self.open_editor(window, cx),
            "up" | "down" | "left" | "right" | "home" | "end" => {
                let key = e.keystroke.key.as_str();
                if !self.slider && matches!(key, "left" | "right" | "home" | "end") {
                    return;
                }
                let delta = if matches!(key, "down" | "left") {
                    -1.
                } else {
                    1.
                } * self.options.step()
                    * if e.keystroke.modifiers.shift { 10. } else { 1. };
                if self.editing {
                    let draft = self.input.read(cx).value().parse::<f64>().unwrap_or(0.);
                    let n = logic::round(draft + delta, 4);
                    self.input
                        .update(cx, |input, cx| input.set_value(n.to_string(), cx));
                } else {
                    let n = match key {
                        "home" => self.options.min,
                        "end" => self.options.max,
                        _ => self.state.current(cx) + delta,
                    };
                    let n = if self.slider {
                        logic::snap(
                            n,
                            self.options.min,
                            self.options.max,
                            self.options.step().abs(),
                        )
                    } else {
                        self.options.clamp(n)
                    };
                    self.state.once(n, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
        let _ = window;
    }
    fn scroll(&mut self, e: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled || self.editing || self.drag.is_some() {
            return;
        }
        let delta = e.delta.pixel_delta(px(16.));
        let now = self.wheel_epoch.elapsed().as_secs_f64() * 1000.;
        let Some(delta) = self.wheel.delta(
            -f64::from(delta.x),
            -f64::from(delta.y),
            e.modifiers.control,
            now,
        ) else {
            return;
        };
        if !self.state.gesture.active() && !self.state.begin(cx) {
            self.wheel.finish();
            return;
        }
        if delta != 0. {
            let n = self.options.clamp(
                self.state.value
                    + delta
                        * self.options.step()
                        * self.options.speed()
                        * logic::multiplier(e.modifiers.shift, e.modifiers.alt),
            );
            let changed = n != self.state.value;
            if self.wheel.haptic(delta, changed, now) {
                crate::native::alignment();
            }
            self.state.change(n, cx);
        }
        self.wheel_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.wheel.finish();
                this.state.commit(cx);
                cx.notify();
            });
        }));
        window.prevent_default();
        cx.stop_propagation();
        cx.notify();
    }
}
impl Focusable for NumField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.state.focus.clone()
    }
}
impl Render for NumField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.state.theme(window);
        let value = if self.state.gesture.active() {
            self.state.value
        } else {
            self.state.current(cx)
        };
        let fraction = ((value - self.options.min) / (self.options.max - self.options.min))
            .clamp(0., 1.) as f32;
        self.input
            .update(cx, |i, _| i.appearance = self.state.appearance);
        let text = if self.editing {
            self.input.clone().into_any_element()
        } else {
            div()
                .child(if self.state.mixed {
                    "Mixed".to_string()
                } else {
                    self.options.format(value)
                })
                .into_any_element()
        };
        let mut well = self
            .state
            .well(window)
            .relative()
            .role(Role::SpinButton)
            .aria_numeric_value(value)
            .aria_numeric_value_step(self.options.step())
            .aria_value(if self.state.mixed {
                "Mixed".to_string()
            } else {
                self.options.format(value)
            })
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, w, cx| {
                if !this.editing || matches!(e.keystroke.key.as_str(), "up" | "down" | "escape") {
                    this.key(e, w, cx);
                }
            }))
            .on_scroll_wheel(cx.listener(Self::scroll));
        if self.options.min.is_finite() {
            well = well.aria_min_numeric_value(self.options.min);
        }
        if self.options.max.is_finite() {
            well = well.aria_max_numeric_value(self.options.max);
        }
        if self.editing {
            well = well.border_color(t.accent).child(text);
        } else if self.slider {
            let dial = self.angle && self.options.max - self.options.min >= 360.;
            let visual = div()
                .id("slider")
                .role(Role::Slider)
                .aria_label(format!("{} slider", self.state.label))
                .flex_1()
                .h_full()
                .relative()
                .focusable()
                .tab_index(if self.state.disabled { -1 } else { 0 })
                .focus(|d| d.border_1().border_color(t.accent))
                .when(dial, |d| {
                    d.flex_none()
                        .w(px(self.state.size.height() - 4.))
                        .h(px(self.state.size.height() - 4.))
                        .rounded_full()
                        .bg(t.ink_3)
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, e, w, cx| this.down(e, w, cx, true)),
                )
                .child(measure(cx.entity(), |this, b| this.bounds = b));
            well = well
                .gap_2()
                .child(
                    div()
                        .w(px(62.))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, e, w, cx| this.down(e, w, cx, false)),
                        )
                        .child(text),
                )
                .child(
                    visual
                        .when(
                            self.angle && self.options.max - self.options.min >= 360.,
                            |d| {
                                d.child(
                                    canvas(
                                        |_, _, _| (),
                                        move |bounds, _, window, _| {
                                            let center = bounds.center();
                                            let angle = value.to_radians();
                                            let r = bounds.size.width * 0.35;
                                            let end = point(
                                                center.x + r * angle.cos() as f32,
                                                center.y + r * angle.sin() as f32,
                                            );
                                            let mut path = PathBuilder::stroke(px(1.5));
                                            path.move_to(center);
                                            path.line_to(end);
                                            if let Ok(path) = path.build() {
                                                window.paint_path(path, t.text);
                                            }
                                        },
                                    )
                                    .size_full(),
                                )
                            },
                        )
                        .when(!dial, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top(px(10.))
                                    .h(px(3.))
                                    .w(relative(fraction))
                                    .rounded_full()
                                    .bg(t.accent),
                            )
                        }),
                );
        } else {
            well = well
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, e, w, cx| this.down(e, w, cx, false)),
                )
                .child(text);
        }
        well.when(self.drag.is_some(), |d| {
            d.child(crate::field::drag_listeners(
                cx.entity(),
                |this, e, _, cx| this.move_drag(e, cx),
                Self::up,
            ))
        })
    }
}
