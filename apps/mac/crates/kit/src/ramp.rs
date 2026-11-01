use crate::{
    color::Color,
    field::{FieldState, drag_listeners, field_api, measure},
    logic,
};
use gpui::{prelude::*, *};
#[derive(Clone, Debug, PartialEq)]
pub struct FillStop {
    pub id: String,
    pub color: Color,
    pub position: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RampValue {
    pub stops: Vec<FillStop>,
    pub midpoint: f64,
}
impl Default for RampValue {
    fn default() -> Self {
        Self {
            stops: vec![
                FillStop {
                    id: "stop-1".into(),
                    color: Color::new(1., 0.35, 0., 1.),
                    position: 0.,
                },
                FillStop {
                    id: "stop-2".into(),
                    color: Color::new(0.2, 0.4, 1., 1.),
                    position: 100.,
                },
            ],
            midpoint: 50.,
        }
    }
}
#[derive(Clone, Copy)]
enum Target {
    Stop(usize),
    Midpoint,
}
pub struct RampField {
    pub state: FieldState<RampValue>,
    pub selected: usize,
    pub show_midpoint: bool,
    bounds: Bounds<Pixels>,
    drag: Option<Target>,
    on_select: Option<std::rc::Rc<dyn Fn(&str, &mut App)>>,
}
impl RampField {
    pub fn new(value: RampValue, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = FieldState::new(value, "Gradient", cx);
        cx.on_release(|this, cx| this.state.cancel(cx)).detach();
        cx.observe_window_activation(window, |this, w, cx| {
            if !w.is_window_active() {
                this.drag = None;
                this.state.cancel(cx);
                cx.notify();
            }
        })
        .detach();
        Self {
            state,
            selected: 0,
            show_midpoint: true,
            bounds: Bounds::default(),
            drag: None,
            on_select: None,
        }
    }
    field_api!(RampValue);
    pub fn on_select(mut self, f: impl Fn(&str, &mut App) + 'static) -> Self {
        self.on_select = Some(std::rc::Rc::new(f));
        self
    }
    fn modify(&self, mut value: RampValue, target: Target, position: f64) -> RampValue {
        match target {
            Target::Stop(i) => {
                if let Some(stop) = value.stops.get_mut(i) {
                    stop.position = logic::round(position.clamp(0., 100.), 1);
                }
            }
            Target::Midpoint => value.midpoint = position.clamp(1., 99.),
        }
        value
    }
    fn at(&mut self, p: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(target) = self.drag {
            let n = f64::from((p.x - self.bounds.left()) / self.bounds.size.width) * 100.;
            let value = self.modify(self.state.value.clone(), target, n);
            self.state.change(value, cx);
            cx.notify();
        }
    }
    fn up(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.state.commit(cx);
            cx.notify();
        }
    }
    fn down(&mut self, target: Target, e: &MouseDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            return;
        }
        if let Target::Stop(i) = target {
            self.selected = i;
            if let (Some(f), Some(stop)) = (&self.on_select, self.state.value.stops.get(i)) {
                f(&stop.id, cx);
            }
        }
        if self.state.begin(cx) {
            self.drag = Some(target);
            self.state.focus.focus(w, cx);
            self.at(e.position, cx);
        }
        cx.stop_propagation();
    }
    fn key(&mut self, target: Target, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = e.keystroke.key.as_str();
        if key == "escape" {
            self.drag = None;
            self.state.cancel(cx);
        } else {
            let current = self.state.current(cx);
            let position = match target {
                Target::Stop(i) => current.stops.get(i).map(|s| s.position).unwrap_or(0.),
                Target::Midpoint => current.midpoint,
            };
            let n = match key {
                "home" => 0.,
                "end" => 100.,
                "left" => position - if e.keystroke.modifiers.shift { 10. } else { 1. },
                "right" => position + if e.keystroke.modifiers.shift { 10. } else { 1. },
                _ => return,
            };
            self.state.once(self.modify(current, target, n), cx);
        }
        cx.stop_propagation();
        cx.notify();
    }
}
/// Evaluate sorted stops without mutating authored order. Midpoint applies to two-stop ramps.
pub fn sample(value: &RampValue, fraction: f64) -> Color {
    let mut stops = value.stops.iter().collect::<Vec<_>>();
    stops.sort_by(|a, b| a.position.total_cmp(&b.position));
    let Some(first) = stops.first() else {
        return Color::default();
    };
    let p = fraction.clamp(0., 1.) * 100.;
    if p <= first.position {
        return first.color;
    }
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if p == b.position {
            return b.color;
        }
        if p < b.position {
            let mut t = ((p - a.position) / (b.position - a.position).max(1e-8)).clamp(0., 1.);
            if stops.len() == 2 {
                let mid = (value.midpoint / 100.).clamp(0.01, 0.99);
                t = if t <= mid {
                    t / mid * 0.5
                } else {
                    0.5 + (t - mid) / (1. - mid) * 0.5
                };
            }
            let mix = |x: f64, y: f64| x + (y - x) * t;
            return Color::new(
                mix(a.color.r, b.color.r),
                mix(a.color.g, b.color.g),
                mix(a.color.b, b.color.b),
                mix(a.color.a, b.color.a),
            );
        }
    }
    stops.last().unwrap().color
}
impl Focusable for RampField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.state.focus.clone()
    }
}
impl Render for RampField {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.state.theme(w);
        let value = self.state.value.clone();
        let mut root = self
            .state
            .well(w)
            .role(Role::Group)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" {
                    this.key(Target::Stop(this.selected), e, w, cx);
                }
            }))
            .relative()
            .h(px(44.))
            .px_0()
            .bg(t.sunken)
            .child(measure(cx.entity(), |this, b| this.bounds = b))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(10.))
                    .h(px(20.))
                    .rounded(px(4.))
                    .overflow_hidden()
                    .flex()
                    .children((0..80).map(|i| {
                        div()
                            .flex_1()
                            .h_full()
                            .bg(sample(&value, i as f64 / 79.).rgba())
                    })),
            );
        for (i, stop) in value.stops.iter().enumerate() {
            let target = Target::Stop(i);
            root = root.child(
                div()
                    .id(i)
                    .role(Role::Slider)
                    .aria_label(format!("Color stop {}", i + 1))
                    .aria_numeric_value(stop.position)
                    .aria_min_numeric_value(0.)
                    .aria_max_numeric_value(100.)
                    .tab_index(if self.state.disabled { -1 } else { 0 })
                    .focusable()
                    .absolute()
                    .left(relative((stop.position / 100.).clamp(0., 1.) as f32))
                    .ml(px(-5.))
                    .top(px(25.))
                    .size(px(12.))
                    .rounded(px(3.))
                    .border_2()
                    .border_color(if self.selected == i { t.accent } else { t.text })
                    .bg(stop.color.rgba())
                    .on_key_down(cx.listener(move |this, e, w, cx| this.key(target, e, w, cx)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e, w, cx| this.down(target, e, w, cx)),
                    ),
            );
        }
        if self.show_midpoint && value.stops.len() == 2 {
            root = root.child(
                div()
                    .id("midpoint")
                    .role(Role::Slider)
                    .aria_label("Gradient midpoint")
                    .aria_numeric_value(value.midpoint)
                    .aria_min_numeric_value(1.)
                    .aria_max_numeric_value(99.)
                    .tab_index(if self.state.disabled { -1 } else { 0 })
                    .focusable()
                    .absolute()
                    .left(relative((value.midpoint / 100.) as f32))
                    .ml(px(-5.))
                    .top(px(-3.))
                    .text_color(t.text)
                    .child("◆")
                    .on_key_down(cx.listener(|this, e, w, cx| this.key(Target::Midpoint, e, w, cx)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, e, w, cx| this.down(Target::Midpoint, e, w, cx)),
                    ),
            );
        }
        root.when(self.drag.is_some(), |d| {
            d.child(drag_listeners(
                cx.entity(),
                |this, e, _, cx| this.at(e.position, cx),
                Self::up,
            ))
        })
    }
}
#[cfg(test)]
mod tests {
    use super::{RampValue, sample};
    #[test]
    fn midpoint_and_endpoints() {
        let mut r = RampValue::default();
        assert_eq!(sample(&r, 0.), r.stops[0].color);
        assert_eq!(sample(&r, 1.), r.stops[1].color);
        r.midpoint = 25.;
        let c = sample(&r, 0.25);
        assert!((c.r - 0.6).abs() < 1e-8);
        r.stops.reverse();
        assert_eq!(sample(&r, 0.), r.stops[1].color);
    }
}
