//! Shared color picker. Wide gamut drafts stay wide until the document callback commits.
use crate::{
    ActionButton, NumField, Row,
    color::{self, Color, Notation},
    field::{FieldState, drag_listeners, field_api, measure},
    input::{InputEvent, TextInput},
    logic::EditEvent,
};
use gpui::{prelude::*, *};
pub struct ColorField {
    pub state: FieldState<Color>,
    pub embedded: bool,
    open: bool,
    settings: bool,
    p3: bool,
    notation: Notation,
    before: Color,
    bounds: Bounds<Pixels>,
    pad: Bounds<Pixels>,
    popup_focus: FocusHandle,
    dragging: bool,
    channels: [Entity<NumField>; 4],
    text: Entity<TextInput>,
    on_sample: Option<std::rc::Rc<dyn Fn(&mut Window, &mut App) -> Option<Color>>>,
}
impl ColorField {
    pub fn new(value: Color, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = FieldState::new(value, "Color", cx);
        let channels = std::array::from_fn(|axis| {
            let weak = cx.weak_entity();
            cx.new(|cx| {
                NumField::slider(0., 0., if axis == 0 { 360. } else { 100. }, window, cx)
                    .label(["Hue", "Saturation", "Brightness", "Opacity"][axis])
                    .unit(if axis == 0 { "°" } else { "%" })
                    .on_edit(move |event, cx| {
                        let _ = weak.update(cx, |this: &mut Self, cx| {
                            if let EditEvent::Change(v) | EditEvent::Cancel(v) = event {
                                this.axis(axis, *v, cx);
                            }
                            cx.notify();
                        });
                    })
            })
        });
        let text = cx.new(|cx| TextInput::new(value.hex(), "Color value", window, cx));
        cx.subscribe_in(&text, window, |this, _, event, _, cx| {
            match event {
                InputEvent::Change(_) => {}
                InputEvent::Commit => this.apply_text(cx),
                InputEvent::Cancel => this.finish(false, None, cx),
            }
            cx.notify();
        })
        .detach();
        cx.on_release(|this, cx| this.state.cancel(cx)).detach();
        cx.observe_window_activation(window, |this, w, cx| {
            if !w.is_window_active() {
                this.dragging = false;
                this.finish(false, None, cx);
            }
        })
        .detach();
        Self {
            state,
            embedded: false,
            open: false,
            settings: false,
            p3: false,
            notation: Notation::Hex,
            before: value,
            bounds: Bounds::default(),
            pad: Bounds::default(),
            popup_focus: cx.focus_handle(),
            dragging: false,
            channels,
            text,
            on_sample: None,
        }
    }
    field_api!(Color);
    pub fn embedded(mut self, v: bool) -> Self {
        self.embedded = v;
        self
    }
    /// Supply AppKit's screen sampler from the chrome bridge. No document writes occur here.
    pub fn on_sample(
        mut self,
        f: impl Fn(&mut Window, &mut App) -> Option<Color> + 'static,
    ) -> Self {
        self.on_sample = Some(std::rc::Rc::new(f));
        self
    }
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.finish(false, None, cx);
    }
    pub(crate) fn pending_color(&self, cx: &App) -> Color {
        let raw = self.text.read(cx).value();
        let source = match self.notation {
            Notation::Hex => raw.to_string(),
            Notation::Rgb => format!("rgb({raw})"),
            Notation::Hsl => format!("hsl({raw})"),
            Notation::Oklch => format!("oklch({raw})"),
            Notation::DisplayP3 => format!("color(display-p3 {raw})"),
        };
        color::parse(&source)
            .map(|mut c| {
                c.a = self.state.value.a;
                c
            })
            .unwrap_or(self.state.value)
    }
    pub(crate) fn reset_draft(&mut self, value: Color, cx: &mut Context<Self>) {
        self.state.gesture = Default::default();
        self.state.value = value;
        self.before = value;
        self.sync_text(cx);
        cx.notify();
    }
    pub fn commit(&mut self, cx: &mut Context<Self>) {
        self.apply_text(cx);
        self.finish(true, None, cx);
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
            self.apply_text(cx);
            self.finish(true, Some(w), cx);
            return;
        }
        self.before = self.state.current(cx);
        self.state.value = self.before;
        self.open = true;
        self.state.focus.focus(w, cx);
        self.sync_text(cx);
        cx.notify();
    }
    fn change(&mut self, color: Color, cx: &mut Context<Self>) {
        if self.state.disabled || (!self.open && !self.embedded) {
            return;
        }
        if !self.state.gesture.active() && !self.state.begin(cx) {
            return;
        }
        self.state.change(color, cx);
        self.sync_text(cx);
        cx.notify();
    }
    fn sync_text(&mut self, cx: &mut Context<Self>) {
        let text = self.state.value.format(self.notation);
        self.text.update(cx, |i, cx| i.set_value(text, cx));
    }
    fn apply_text(&mut self, cx: &mut Context<Self>) {
        let raw = self.text.read(cx).value().to_string();
        let source = match self.notation {
            Notation::Hex => raw,
            Notation::Rgb => format!("rgb({raw})"),
            Notation::Hsl => format!("hsl({raw})"),
            Notation::Oklch => format!("oklch({raw})"),
            Notation::DisplayP3 => format!("color(display-p3 {raw})"),
        };
        if let Some(mut color) = color::parse(&source) {
            color.a = self.state.value.a;
            self.change(color, cx);
        }
    }
    fn finish(&mut self, commit: bool, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.dragging = false;
        if commit {
            if self.state.gesture.active() {
                let normalized =
                    color::parse(&self.state.value.stored_hex()).unwrap_or(self.state.value);
                self.state.change(normalized, cx);
                self.state.commit(cx);
            }
        } else {
            self.state.cancel(cx);
        }
        self.open = false;
        if let Some(w) = window {
            self.state.focus.focus(w, cx);
        }
        cx.notify();
    }
    fn working(&self) -> Color {
        if self.p3 {
            self.state.value.p3()
        } else {
            self.state.value.clipped()
        }
    }
    fn axis(&mut self, axis: usize, n: f64, cx: &mut Context<Self>) {
        let current = self.state.value;
        let (h, s, v) = self.working().hsv();
        let color = match axis {
            0 => Color::from_hsv(n, s, v, current.a),
            1 => Color::from_hsv(h, n / 100., v, current.a),
            2 => Color::from_hsv(h, s, n / 100., current.a),
            _ => Color {
                a: n / 100.,
                ..self.working()
            },
        };
        self.change(
            if self.p3 {
                Color::from_p3(color)
            } else {
                color
            },
            cx,
        );
    }
    fn at(&mut self, p: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        let s = f64::from((p.x - self.pad.left()) / self.pad.size.width).clamp(0., 1.);
        let v = 1. - f64::from((p.y - self.pad.top()) / self.pad.size.height).clamp(0., 1.);
        let (h, _, _) = self.working().hsv();
        let color = Color::from_hsv(h, s, v, self.state.value.a);
        self.change(
            if self.p3 {
                Color::from_p3(color)
            } else {
                color
            },
            cx,
        );
    }
    fn up(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.dragging = false;
        cx.notify();
    }
    fn key(&mut self, e: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        match e.keystroke.key.as_str() {
            "escape" => {
                self.finish(false, Some(w), cx);
                cx.stop_propagation();
            }
            "tab" if self.open && !self.embedded => {
                if e.keystroke.modifiers.shift {
                    w.focus_prev(cx);
                } else {
                    w.focus_next(cx);
                }
                if !self.popup_focus.contains_focused(w, cx) {
                    if e.keystroke.modifiers.shift {
                        self.text.read(cx).focus_handle(cx).focus(w, cx);
                    } else {
                        self.channels[0].read(cx).focus_handle(cx).focus(w, cx);
                    }
                }
                cx.stop_propagation();
            }
            "enter" => {
                if !self.open && !self.embedded {
                    self.show(w, cx);
                } else {
                    self.apply_text(cx);
                    self.finish(true, Some(w), cx);
                }
                cx.stop_propagation();
            }
            _ => {}
        }
    }
    fn pad_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (h, mut s, mut v) = self.working().hsv();
        let step = if e.keystroke.modifiers.shift {
            0.1
        } else {
            0.01
        };
        match e.keystroke.key.as_str() {
            "left" => s -= step,
            "right" => s += step,
            "up" => v += step,
            "down" => v -= step,
            "home" => {
                s = 0.;
                v = 0.;
            }
            "end" => {
                s = 1.;
                v = 1.;
            }
            _ => return,
        }
        let c = Color::from_hsv(h, s.clamp(0., 1.), v.clamp(0., 1.), self.state.value.a);
        self.change(if self.p3 { Color::from_p3(c) } else { c }, cx);
        cx.stop_propagation();
    }
}
impl Focusable for ColorField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.state.focus.clone()
    }
}
impl Render for ColorField {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.state.theme(w);
        let current = if self.state.gesture.active() {
            self.state.value
        } else {
            self.state.current(cx)
        };
        if !self.state.gesture.active() {
            self.state.value = current;
        }
        let (h, s, v) = self.working().hsv();
        for (i, value) in [h, s * 100., v * 100., self.state.value.a * 100.]
            .into_iter()
            .enumerate()
        {
            self.channels[i].update(cx, |n, cx| {
                n.state.appearance = self.state.appearance;
                n.state.disabled = self.state.disabled;
                n.set_value(value, cx);
            });
        }
        self.text.update(cx, |i, _| {
            i.appearance = self.state.appearance;
            i.disabled = self.state.disabled;
        });
        let mut root = div()
            .relative()
            .w_full()
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, w, cx| {
                if matches!(e.keystroke.key.as_str(), "escape" | "tab") {
                    this.key(e, w, cx);
                }
            }))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "enter" {
                    this.key(e, w, cx);
                }
            }));
        if !self.embedded {
            root = root.child(
                self.state
                    .well(w)
                    .role(Role::Button)
                    .aria_expanded(self.open)
                    .on_click(cx.listener(|this, _, w, cx| this.show(w, cx)))
                    .child(div().size(px(16.)).rounded(px(4.)).bg(current.rgba()))
                    .child(div().pl_2().child(if self.state.mixed {
                        "Mixed".to_string()
                    } else {
                        current.stored_hex()
                    }))
                    .child(measure(cx.entity(), |this, b| this.bounds = b)),
            );
        }
        if self.open || self.embedded {
            let weak = cx.weak_entity();
            let restore = weak.clone();
            let settings = weak.clone();
            let mut picker = div()
                .id("color-picker")
                .track_focus(&self.popup_focus)
                .role(Role::Dialog)
                .aria_label(self.state.label.clone())
                .flex()
                .flex_col()
                .gap_2()
                .w(px(264.))
                .p_3()
                .rounded(px(12.))
                .bg(t.float)
                .border_1()
                .border_color(t.line_strong)
                .text_color(t.text)
                .text_size(px(12.))
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _, w, cx| {
                    if !this.embedded {
                        this.apply_text(cx);
                        this.finish(true, Some(w), cx);
                    }
                    let _ = w;
                }));
            picker = picker.child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child("Color")
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                ActionButton::new("restore", "↶", move |_, cx| {
                                    let _ =
                                        restore.update(cx, |this, cx| this.change(this.before, cx));
                                })
                                .in_focus_scope(self.popup_focus.clone())
                                .appearance(self.state.appearance),
                            )
                            .child(
                                ActionButton::new("settings", "•••", move |_, cx| {
                                    let _ = settings.update(cx, |this, cx| {
                                        this.settings = !this.settings;
                                        cx.notify();
                                    });
                                })
                                .in_focus_scope(self.popup_focus.clone())
                                .appearance(self.state.appearance),
                            ),
                    ),
            );
            if self.settings {
                let weak = weak.clone();
                let copy = weak.clone();
                let sample = weak.clone();
                picker = picker.child(
                    div()
                        .flex()
                        .gap_1()
                        .child(
                            ActionButton::new(
                                "space",
                                if self.p3 { "Display P3" } else { "sRGB" },
                                move |_, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.p3 = !this.p3;
                                        cx.notify();
                                    });
                                },
                            )
                            .in_focus_scope(self.popup_focus.clone())
                            .appearance(self.state.appearance),
                        )
                        .child(
                            ActionButton::new("copy", "Copy", move |_, cx| {
                                let _ = copy.update(cx, |this, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        this.state.value.stored_hex(),
                                    ))
                                });
                            })
                            .in_focus_scope(self.popup_focus.clone())
                            .appearance(self.state.appearance),
                        )
                        .child(
                            ActionButton::new("sample", "Sample", move |w, cx| {
                                let _ = sample.update(cx, |this, cx| {
                                    if let Some(f) = &this.on_sample {
                                        if let Some(c) = f(w, cx) {
                                            this.change(c, cx);
                                        }
                                    }
                                });
                            })
                            .disabled(self.on_sample.is_none())
                            .in_focus_scope(self.popup_focus.clone())
                            .appearance(self.state.appearance),
                        ),
                );
            }
            let hue = h;
            let mut pad = div()
                .id("saturation-brightness")
                .debug_selector(|| "color-pad".to_string())
                .role(Role::Slider)
                .aria_label("Saturation and brightness")
                .aria_value(format!("{}%, {}%", (s * 100.).round(), (v * 100.).round()))
                .tab_index(if self.state.disabled { -1 } else { 0 })
                .focusable()
                .relative()
                .w_full()
                .h(px(132.))
                .rounded(px(6.))
                .overflow_hidden()
                .cursor(CursorStyle::Crosshair)
                .on_key_down(cx.listener(Self::pad_key))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, e: &MouseDownEvent, _, cx| {
                        if !this.state.disabled {
                            this.dragging = true;
                            this.at(e.position, cx);
                            cx.stop_propagation();
                        }
                    }),
                );
            for y in 0..20 {
                pad = pad.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(relative(y as f32 / 20.))
                        .h(relative(1. / 20.))
                        .flex()
                        .children((0..40).map(move |x| {
                            let c = Color::from_hsv(hue, x as f64 / 39., 1. - y as f64 / 19., 1.);
                            div().flex_1().h_full().bg(c.rgba())
                        })),
                );
            }
            pad = pad
                .child(measure(cx.entity(), |this, b| this.pad = b))
                .child(
                    div()
                        .absolute()
                        .left(relative(s.clamp(0., 1.) as f32))
                        .top(relative((1. - v).clamp(0., 1.) as f32))
                        .ml(px(-4.))
                        .mt(px(-4.))
                        .size(px(9.))
                        .rounded_full()
                        .border_2()
                        .border_color(gpui::white()),
                );
            picker = picker
                .child(pad)
                .children(self.channels.iter().enumerate().map(|(i, c)| {
                    Row::new(["Hue", "Saturation", "Brightness", "Opacity"][i])
                        .appearance(self.state.appearance)
                        .child(c.clone())
                }));
            let next_notation = weak.clone();
            picker = picker.child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        ActionButton::new(
                            "notation",
                            format!("{:?}", self.notation),
                            move |_, cx| {
                                let _ = next_notation.update(cx, |this, cx| {
                                    this.notation = match this.notation {
                                        Notation::Hex => Notation::Rgb,
                                        Notation::Rgb => Notation::Hsl,
                                        Notation::Hsl => Notation::Oklch,
                                        Notation::Oklch => Notation::DisplayP3,
                                        Notation::DisplayP3 => Notation::Hex,
                                    };
                                    this.sync_text(cx);
                                    cx.notify();
                                });
                            },
                        )
                        .in_focus_scope(self.popup_focus.clone())
                        .appearance(self.state.appearance),
                    )
                    .child(div().flex_1().child(self.text.clone())),
            );
            if self.state.value.outside_srgb() {
                picker = picker.child(
                    div()
                        .text_color(t.warning)
                        .child("Clipped · saves nearest sRGB color"),
                );
            }
            if self.dragging {
                picker = picker.child(drag_listeners(
                    cx.entity(),
                    |this, e, _, cx| this.at(e.position, cx),
                    Self::up,
                ));
            }
            if self.embedded {
                root = root.child(picker);
            } else {
                root = root.child(
                    deferred(
                        anchored()
                            .position(point(self.bounds.right(), self.bounds.bottom() + px(6.)))
                            .anchor(Anchor::TopRight)
                            .snap_to_window_with_margin(px(8.))
                            .child(picker),
                    )
                    .with_priority(110),
                );
            }
        }
        root
    }
}
