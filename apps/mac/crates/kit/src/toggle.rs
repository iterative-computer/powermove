use crate::field::{FieldState, field_api};
use gpui::{prelude::*, *};
#[derive(Clone, Copy, Debug, Default)]
pub enum ToggleVariant {
    #[default]
    Segmented,
    Checkbox,
}
pub struct ToggleField {
    pub state: FieldState<bool>,
    pub variant: ToggleVariant,
}
impl ToggleField {
    pub fn new(value: bool, _: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            state: FieldState::new(value, "Toggle", cx),
            variant: Default::default(),
        }
    }
    field_api!(bool);
    pub fn variant(mut self, variant: ToggleVariant) -> Self {
        self.variant = variant;
        self
    }
    fn key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let value = match e.keystroke.key.as_str() {
            "left" | "up" => false,
            "right" | "down" => true,
            "space" | "enter" => self.state.mixed || !self.state.current(cx),
            _ => return,
        };
        self.state.once(value, cx);
        cx.stop_propagation();
        cx.notify();
    }
}
impl Focusable for ToggleField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.state.focus.clone()
    }
}
impl Render for ToggleField {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.state.theme(w);
        let value = self.state.current(cx);
        let mixed = self.state.mixed;
        let root = self
            .state
            .well(w)
            .role(Role::CheckBox)
            .aria_toggled(if mixed {
                Toggled::Mixed
            } else if value {
                Toggled::True
            } else {
                Toggled::False
            })
            .on_key_down(cx.listener(Self::key));
        match self.variant {
            ToggleVariant::Checkbox => root
                .w(px(20.))
                .h(px(20.))
                .px_0()
                .justify_center()
                .when(value && !mixed, |d| d.bg(t.accent).text_color(t.on_accent))
                .child(if mixed {
                    "−"
                } else if value {
                    "✓"
                } else {
                    ""
                })
                .on_click(cx.listener(|this, _, w, cx| {
                    if this.state.disabled {
                        return;
                    }
                    w.blur(cx);
                    let v = this.state.mixed || !this.state.current(cx);
                    this.state.once(v, cx);
                    cx.notify();
                })),
            ToggleVariant::Segmented => {
                root.px(px(2.)).gap(px(2.)).children([false, true].map(|v| {
                    div()
                        .id(if v { "on" } else { "off" })
                        .flex_1()
                        .flex()
                        .justify_center()
                        .items_center()
                        .h(px(self.state.size.height() - 4.))
                        .rounded(px((self.state.size.radius() - 2.).max(2.)))
                        .when(!mixed && v == value, |d| d.bg(t.ink_3).text_color(t.text))
                        .child(if v { "On" } else { "Off" })
                        .on_click(cx.listener(move |this, _, w, cx| {
                            if this.state.disabled {
                                return;
                            }
                            w.blur(cx);
                            this.state.once(v, cx);
                            cx.notify();
                        }))
                }))
            }
        }
    }
}
