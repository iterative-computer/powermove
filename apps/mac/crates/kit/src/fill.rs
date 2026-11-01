//! Fill editing shares ColorField and RampField rather than a second picker implementation.
use crate::{
    ActionButton, ColorField, FillStop, NumField, RampField, RampValue, Row,
    color::Color,
    field::{FieldState, field_api, measure},
    logic::EditEvent,
};
use gpui::{prelude::*, *};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillMode {
    Solid,
    Linear,
    Radial,
    None,
}
#[derive(Clone, Debug, PartialEq)]
pub struct FillValue {
    pub mode: FillMode,
    pub angle: f64,
    pub ramp: RampValue,
}
impl Default for FillValue {
    fn default() -> Self {
        Self {
            mode: FillMode::Solid,
            angle: 0.,
            ramp: RampValue::default(),
        }
    }
}
impl FillValue {
    pub fn solid(color: Color) -> Self {
        Self {
            mode: FillMode::Solid,
            angle: 0.,
            ramp: RampValue {
                stops: vec![FillStop {
                    id: "stop-1".into(),
                    color,
                    position: 0.,
                }],
                midpoint: 50.,
            },
        }
    }
    pub fn normalized(&self, fallback: Color) -> Self {
        let mut v = self.clone();
        if !v.angle.is_finite() {
            v.angle = 0.;
        }
        v.ramp.midpoint = if v.ramp.midpoint.is_finite() {
            v.ramp.midpoint.clamp(1., 99.)
        } else {
            50.
        };
        v.ramp.stops.truncate(8);
        for (i, stop) in v.ramp.stops.iter_mut().enumerate() {
            if stop.id.is_empty() {
                stop.id = format!("stop-{}", i + 1);
            }
            stop.position = if stop.position.is_finite() {
                stop.position.clamp(0., 100.)
            } else {
                0.
            };
            stop.color = crate::color::parse(&stop.color.stored_hex()).unwrap_or(fallback);
        }
        if v.ramp.stops.is_empty() {
            v.ramp.stops.push(FillStop {
                id: "stop-1".into(),
                color: fallback,
                position: 0.,
            });
        }
        if matches!(v.mode, FillMode::Linear | FillMode::Radial) && v.ramp.stops.len() == 1 {
            let mut stop = v.ramp.stops[0].clone();
            stop.id = "stop-2".into();
            stop.position = 100.;
            v.ramp.stops.push(stop);
        }
        v
    }
}
pub struct FillField {
    pub state: FieldState<FillValue>,
    pub fallback: Color,
    open: bool,
    bounds: Bounds<Pixels>,
    selected: usize,
    popup_focus: FocusHandle,
    color: Entity<ColorField>,
    ramp: Entity<RampField>,
    angle: Entity<NumField>,
}
impl FillField {
    pub fn new(value: FillValue, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let fallback = Color::default();
        let value = value.normalized(fallback);
        let state = FieldState::new(value.clone(), "Fill", cx);
        let weak = cx.weak_entity();
        let color = cx.new(|cx| {
            ColorField::new(value.ramp.stops[0].color, w, cx)
                .embedded(true)
                .on_change(move |color, cx| {
                    let _ = weak.update(cx, |this: &mut Self, cx| {
                        if !this.open {
                            return;
                        }
                        let mut value = this.state.value.clone();
                        if let Some(stop) = value.ramp.stops.get_mut(this.selected) {
                            stop.color = *color;
                        }
                        this.preview(value, cx);
                    });
                })
        });
        let weak = cx.weak_entity();
        let select = weak.clone();
        let ramp = cx.new(|cx| {
            RampField::new(value.ramp.clone(), w, cx)
                .on_edit(move |event, cx| {
                    let _ = weak.update(cx, |this: &mut Self, cx| {
                        if let EditEvent::Change(ramp) | EditEvent::Cancel(ramp) = event {
                            let mut value = this.state.value.clone();
                            value.ramp = ramp.clone();
                            this.preview(value, cx);
                        }
                    });
                })
                .on_select(move |id, cx| {
                    let _ = select.update(cx, |this: &mut Self, cx| {
                        this.selected = this
                            .state
                            .value
                            .ramp
                            .stops
                            .iter()
                            .position(|s| s.id == id)
                            .unwrap_or(0);
                        cx.notify();
                    });
                })
        });
        let weak = cx.weak_entity();
        let angle = cx.new(|cx| {
            NumField::new(value.angle, w, cx)
                .unit("°")
                .label("Gradient angle")
                .on_change(move |angle, cx| {
                    let _ = weak.update(cx, |this: &mut Self, cx| {
                        let mut value = this.state.value.clone();
                        value.angle = *angle;
                        this.preview(value, cx);
                    });
                })
        });
        cx.on_release(|this, cx| this.state.cancel(cx)).detach();
        cx.observe_window_activation(w, |this, w, cx| {
            if !w.is_window_active() {
                this.finish(false, None, cx);
            }
        })
        .detach();
        Self {
            state,
            fallback,
            open: false,
            bounds: Bounds::default(),
            selected: 0,
            popup_focus: cx.focus_handle(),
            color,
            ramp,
            angle,
        }
    }
    field_api!(FillValue);
    fn preview(&mut self, value: FillValue, cx: &mut Context<Self>) {
        if !self.open || self.state.disabled {
            return;
        }
        if !self.state.gesture.active() && !self.state.begin(cx) {
            return;
        }
        self.state.change(value, cx);
        cx.notify();
    }
    pub fn open_picker(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            self.show(w, cx);
        }
    }
    fn show(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            return;
        }
        if self.open {
            self.finish(true, Some(w), cx);
            return;
        }
        self.state.value = self.state.current(cx).normalized(self.fallback);
        self.selected = 0;
        self.open = true;
        self.state.focus.focus(w, cx);
        cx.notify();
    }
    fn finish(&mut self, commit: bool, w: Option<&mut Window>, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        if commit {
            let color = self.color.read(cx).pending_color(cx);
            if self
                .state
                .value
                .ramp
                .stops
                .get(self.selected)
                .is_some_and(|s| s.color != color)
            {
                let mut v = self.state.value.clone();
                v.ramp.stops[self.selected].color = color;
                self.preview(v, cx);
            }
            if self.state.gesture.active() {
                let v = self.state.value.normalized(self.fallback);
                self.state.change(v, cx);
                self.state.commit(cx);
            }
        } else {
            self.state.cancel(cx);
        }
        self.open = false;
        let color = self
            .state
            .value
            .ramp
            .stops
            .get(self.selected)
            .or(self.state.value.ramp.stops.first())
            .map(|s| s.color)
            .unwrap_or(self.fallback);
        self.color.update(cx, |c, cx| c.reset_draft(color, cx));
        if let Some(w) = w {
            self.state.focus.focus(w, cx);
        }
        cx.notify();
    }
    fn set_mode(&mut self, mode: FillMode, cx: &mut Context<Self>) {
        let mut v = self.state.value.clone();
        v.mode = mode;
        let v = v.normalized(self.fallback);
        self.selected = self.selected.min(v.ramp.stops.len() - 1);
        self.preview(v, cx);
    }
    fn add_stop(&mut self, cx: &mut Context<Self>) {
        let mut v = self.state.value.clone();
        if v.ramp.stops.len() >= 8 {
            return;
        }
        let mut stop = v.ramp.stops[self.selected].clone();
        let mut i = 1;
        while v.ramp.stops.iter().any(|s| s.id == format!("stop-{i}")) {
            i += 1;
        }
        stop.id = format!("stop-{i}");
        stop.position = (stop.position + 10.).min(100.);
        v.ramp.stops.push(stop);
        self.selected = v.ramp.stops.len() - 1;
        self.preview(v, cx);
    }
    fn remove_stop(&mut self, cx: &mut Context<Self>) {
        if self.state.value.ramp.stops.len() <= 2 {
            return;
        }
        let mut v = self.state.value.clone();
        v.ramp.stops.remove(self.selected);
        self.selected = self.selected.min(v.ramp.stops.len() - 1);
        self.preview(v, cx);
    }
}
impl Focusable for FillField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.state.focus.clone()
    }
}
impl Render for FillField {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.state.theme(w);
        let v = if self.state.gesture.active() {
            self.state.value.clone()
        } else {
            self.state.current(cx)
        }
        .normalized(self.fallback);
        let preview = div()
            .w(px(36.))
            .h(px(16.))
            .rounded(px(4.))
            .overflow_hidden()
            .flex()
            .children((0..30).map(|i| {
                let color = match v.mode {
                    FillMode::None => Color::new(0., 0., 0., 0.),
                    FillMode::Solid => v.ramp.stops[0].color,
                    _ => crate::ramp::sample(&v.ramp, i as f64 / 29.),
                };
                div().flex_1().h_full().bg(color.rgba())
            }));
        let mut root = div()
            .relative()
            .w_full()
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" {
                    this.finish(false, Some(w), cx);
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(
                |this, e: &KeyDownEvent, w, cx| match e.keystroke.key.as_str() {
                    "escape" => {
                        this.finish(false, Some(w), cx);
                        cx.stop_propagation();
                    }
                    "enter" => {
                        if this.open {
                            this.finish(true, Some(w), cx);
                        } else {
                            this.show(w, cx);
                        }
                        cx.stop_propagation();
                    }
                    _ => {}
                },
            ))
            .child(
                self.state
                    .well(w)
                    .role(Role::Button)
                    .aria_expanded(self.open)
                    .gap_2()
                    .child(preview)
                    .child(if self.state.mixed {
                        "Mixed".to_string()
                    } else {
                        format!("{:?}", v.mode)
                    })
                    .on_click(cx.listener(|this, _, w, cx| this.show(w, cx)))
                    .child(measure(cx.entity(), |this, b| this.bounds = b)),
            );
        if self.open {
            let selected = self.selected.min(self.state.value.ramp.stops.len() - 1);
            let selected_color = self.state.value.ramp.stops[selected].color;
            self.color.update(cx, |c, cx| {
                c.state.appearance = self.state.appearance;
                if c.state.value != selected_color {
                    c.reset_draft(selected_color, cx);
                }
            });
            self.ramp.update(cx, |r, cx| {
                r.state.appearance = self.state.appearance;
                r.selected = self.selected;
                r.set_value(self.state.value.ramp.clone(), cx);
            });
            self.angle.update(cx, |n, cx| {
                n.state.appearance = self.state.appearance;
                n.set_value(self.state.value.angle, cx);
            });
            let mut popup = div()
                .id("fill-picker")
                .track_focus(&self.popup_focus)
                .role(Role::Dialog)
                .aria_label(self.state.label.clone())
                .w(px(290.))
                .flex()
                .flex_col()
                .p_3()
                .gap_2()
                .rounded(px(12.))
                .bg(t.float)
                .border_1()
                .border_color(t.line_strong)
                .shadow_lg()
                .text_color(t.text)
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _, w, cx| {
                    this.finish(true, Some(w), cx);
                    let _ = w;
                }));
            popup = popup.child(
                div().flex().gap_1().children(
                    [
                        FillMode::Solid,
                        FillMode::Linear,
                        FillMode::Radial,
                        FillMode::None,
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(i, mode)| {
                        let weak = cx.weak_entity();
                        ActionButton::new(i, format!("{mode:?}"), move |_, cx| {
                            let _ = weak.update(cx, |this, cx| this.set_mode(mode, cx));
                        })
                        .selected(self.state.value.mode == mode)
                        .in_focus_scope(self.popup_focus.clone())
                        .appearance(self.state.appearance)
                    }),
                ),
            );
            if matches!(v.mode, FillMode::Linear | FillMode::Radial) {
                let add = cx.weak_entity();
                let remove = add.clone();
                popup = popup
                    .child(self.ramp.clone())
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                ActionButton::new("add", "+ Stop", move |_, cx| {
                                    let _ = add.update(cx, |this, cx| this.add_stop(cx));
                                })
                                .disabled(v.ramp.stops.len() >= 8)
                                .in_focus_scope(self.popup_focus.clone())
                                .appearance(self.state.appearance),
                            )
                            .child(
                                ActionButton::new("remove", "− Stop", move |_, cx| {
                                    let _ = remove.update(cx, |this, cx| this.remove_stop(cx));
                                })
                                .disabled(v.ramp.stops.len() <= 2)
                                .in_focus_scope(self.popup_focus.clone())
                                .appearance(self.state.appearance),
                            ),
                    )
                    .child(
                        Row::new("Angle")
                            .appearance(self.state.appearance)
                            .child(self.angle.clone()),
                    );
            }
            if v.mode != FillMode::None {
                popup = popup.child(self.color.clone());
            }
            root = root.child(
                deferred(
                    anchored()
                        .position(point(self.bounds.right(), self.bounds.bottom() + px(6.)))
                        .anchor(Anchor::TopRight)
                        .snap_to_window_with_margin(px(8.))
                        .child(popup),
                )
                .with_priority(100),
            );
        }
        root
    }
}
#[cfg(test)]
mod tests {
    use super::{FillMode, FillValue};
    use crate::color::Color;
    #[test]
    fn normalization() {
        let mut v = FillValue::solid(Color::new(1., 0., 0., 0.5));
        v.mode = FillMode::Linear;
        v.angle = f64::NAN;
        v.ramp.midpoint = 120.;
        let n = v.normalized(Color::default());
        assert_eq!(n.ramp.stops.len(), 2);
        assert_eq!(n.ramp.stops[1].position, 100.);
        assert_eq!(n.angle, 0.);
        assert_eq!(n.ramp.midpoint, 99.);
        assert_eq!(n.ramp.stops[0].color.stored_hex(), "#FF000080");
    }
}
