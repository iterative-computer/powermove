use crate::{
    field::{FieldState, field_api},
    input::{InputEvent, TextInput},
};
use gpui::{prelude::*, *};
pub struct TextField {
    pub state: FieldState<String>,
    input: Entity<TextInput>,
    editing: bool,
    pub align: TextAlign,
    pub mono: bool,
}
impl TextField {
    pub fn new(value: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value = value.into();
        let state = FieldState::new(value.clone(), "Text", cx);
        let input = cx.new(|cx| TextInput::new(value, "Text", window, cx));
        let focus = input.read(cx).focus_handle(cx);
        cx.on_focus(&focus, window, |this, _, cx| {
            if this.state.disabled {
                return;
            }
            this.editing = this.state.begin(cx);
            let current = this.state.current(cx);
            this.input.update(cx, |i, cx| i.set_value(current, cx));
        })
        .detach();
        cx.subscribe_in(&input, window, |this, _, event, _, cx| {
            match event {
                InputEvent::Change(value) => {
                    if this.editing {
                        this.state.change(value.clone(), cx);
                    }
                }
                InputEvent::Commit => {
                    if this.editing {
                        this.editing = false;
                        this.state.commit(cx);
                    }
                }
                InputEvent::Cancel => {
                    this.editing = false;
                    this.state.cancel(cx);
                    let value = this.state.value.clone();
                    this.input.update(cx, |i, cx| i.set_value(value, cx));
                }
            }
            cx.notify();
        })
        .detach();
        cx.on_release(|this, cx| this.state.cancel(cx)).detach();
        Self {
            state,
            input,
            editing: false,
            align: TextAlign::Right,
            mono: true,
        }
    }
    field_api!(String);
    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }
    pub fn mono(mut self, mono: bool) -> Self {
        self.mono = mono;
        self
    }
}
impl Focusable for TextField {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}
impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let value = if self.state.mixed && !self.editing {
            "Mixed".to_string()
        } else {
            self.state.current(cx)
        };
        self.input.update(cx, |i, cx| {
            i.appearance = self.state.appearance;
            i.label = self.state.label.clone();
            i.disabled = self.state.disabled;
            if !self.editing && i.value() != value {
                i.set_value(value, cx);
            }
        });
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        self.state
            .well(window)
            .when(focused, |d| d.border_color(self.state.theme(window).accent))
            .tab_index(-1)
            .text_align(self.align)
            .when(self.mono, |d| d.font_family("SF Mono"))
            .child(self.input.clone())
    }
}
