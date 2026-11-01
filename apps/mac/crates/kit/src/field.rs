//! Shared control state and edit phases. The kit never owns document history.
use crate::{
    logic::{EditEvent, Gesture},
    theme::{Appearance, ControlSize, Theme},
};
use gpui::{prelude::*, *};
use std::rc::Rc;
pub type EditCallback<T> = Rc<dyn Fn(&EditEvent<T>, &mut App)>;
pub type ValueCallback<T> = Rc<dyn Fn(&T, &mut App)>;
pub struct FieldState<T> {
    pub value: T,
    pub label: SharedString,
    pub mixed: bool,
    pub disabled: bool,
    pub appearance: Appearance,
    pub size: ControlSize,
    pub focus: FocusHandle,
    pub(crate) gesture: Gesture<T>,
    on_edit: Option<EditCallback<T>>,
    on_change: Option<ValueCallback<T>>,
    on_commit: Option<ValueCallback<T>>,
    on_begin: Option<Rc<dyn Fn(&mut App) -> bool>>,
    live: Option<Rc<dyn Fn(&App) -> T>>,
}
impl<T: Clone + PartialEq + 'static> FieldState<T> {
    pub fn new(value: T, label: impl Into<SharedString>, cx: &mut App) -> Self {
        Self {
            value,
            label: label.into(),
            mixed: false,
            disabled: false,
            appearance: Appearance::System,
            size: ControlSize::Small,
            focus: cx.focus_handle(),
            gesture: Gesture::default(),
            on_edit: None,
            on_change: None,
            on_commit: None,
            on_begin: None,
            live: None,
        }
    }
    pub fn current(&self, cx: &App) -> T {
        self.live
            .as_ref()
            .map(|f| f(cx))
            .unwrap_or_else(|| self.value.clone())
    }
    pub fn set_live(&mut self, f: impl Fn(&App) -> T + 'static) {
        self.live = Some(Rc::new(f));
    }
    pub fn on_edit(&mut self, f: impl Fn(&EditEvent<T>, &mut App) + 'static) {
        self.on_edit = Some(Rc::new(f));
    }
    pub fn on_change(&mut self, f: impl Fn(&T, &mut App) + 'static) {
        self.on_change = Some(Rc::new(f));
    }
    pub fn on_commit(&mut self, f: impl Fn(&T, &mut App) + 'static) {
        self.on_commit = Some(Rc::new(f));
    }
    pub fn on_begin(&mut self, f: impl Fn(&mut App) -> bool + 'static) {
        self.on_begin = Some(Rc::new(f));
    }
    fn emit(&self, e: EditEvent<T>, cx: &mut App) {
        if let Some(f) = &self.on_edit {
            f(&e, cx);
        }
        match &e {
            EditEvent::Change(v) | EditEvent::Cancel(v) => {
                if let Some(f) = &self.on_change {
                    f(v, cx);
                }
            }
            EditEvent::Commit(v) => {
                if let Some(f) = &self.on_commit {
                    f(v, cx);
                }
            }
            _ => {}
        }
    }
    pub fn begin(&mut self, cx: &mut App) -> bool {
        if self.disabled || self.gesture.active() {
            return false;
        }
        if self.on_begin.as_ref().is_some_and(|f| !f(cx)) {
            return false;
        }
        self.value = self.current(cx);
        if self.gesture.begin(&self.value) {
            self.emit(EditEvent::Begin, cx);
            true
        } else {
            false
        }
    }
    pub fn change(&mut self, value: T, cx: &mut App) {
        if self.disabled || !self.gesture.active() {
            return;
        }
        self.gesture.write(&value);
        self.value = value.clone();
        self.emit(EditEvent::Change(value), cx);
    }
    pub fn commit(&mut self, cx: &mut App) {
        if let Some(e) = self.gesture.finish(&self.value) {
            self.emit(e, cx);
        }
    }
    pub fn cancel(&mut self, cx: &mut App) {
        if let Some(e) = self.gesture.cancel() {
            if let EditEvent::Cancel(v) = &e {
                self.value = v.clone();
            }
            self.emit(e, cx);
        }
    }
    pub fn once(&mut self, value: T, cx: &mut App) {
        if !self.mixed && self.current(cx) == value {
            return;
        }
        if self.begin(cx) {
            self.change(value, cx);
            self.commit(cx);
            self.mixed = false;
        }
    }
    pub fn theme(&self, window: &Window) -> Theme {
        Theme::for_window(self.appearance, window)
    }
    pub fn well(&self, window: &Window) -> Stateful<Div> {
        let t = self.theme(window);
        div()
            .id("field")
            .flex()
            .items_center()
            .w_full()
            .min_w(px(32.))
            .h(px(self.size.height()))
            .px(px(7.))
            .rounded(px(self.size.radius()))
            .bg(t.field)
            .text_color(t.value)
            .text_size(px(self.size.font_size()))
            .font_family(crate::theme::SYSTEM_FONT)
            .border_1()
            .border_color(if self.focus.is_focused(window) {
                t.accent
            } else {
                gpui::transparent_black().into()
            })
            .when(self.disabled, |d| d.opacity(0.5))
            .track_focus(&self.focus)
            .tab_index(if self.disabled { -1 } else { 0 })
            .aria_label(self.label.clone())
    }
}
// Common typed API, shared by concrete entities. A live getter is optional for animated readouts.
macro_rules! field_api {
    ($value:ty) => {
        pub fn value(&self) -> &$value {
            &self.state.value
        }
        pub fn set_value(&mut self, value: $value, cx: &mut gpui::Context<Self>) {
            if !self.state.gesture.active() {
                self.state.value = value;
                cx.notify();
            }
        }
        pub fn label(mut self, label: impl Into<gpui::SharedString>) -> Self {
            self.state.label = label.into();
            self
        }
        pub fn disabled(mut self, disabled: bool) -> Self {
            self.state.disabled = disabled;
            self
        }
        pub fn mixed(mut self, mixed: bool) -> Self {
            self.state.mixed = mixed;
            self
        }
        pub fn appearance(mut self, appearance: crate::theme::Appearance) -> Self {
            self.state.appearance = appearance;
            self
        }
        pub fn size(mut self, size: crate::theme::ControlSize) -> Self {
            self.state.size = size;
            self
        }
        pub fn on_edit(
            mut self,
            f: impl Fn(&crate::logic::EditEvent<$value>, &mut gpui::App) + 'static,
        ) -> Self {
            self.state.on_edit(f);
            self
        }
        pub fn on_change(mut self, f: impl Fn(&$value, &mut gpui::App) + 'static) -> Self {
            self.state.on_change(f);
            self
        }
        pub fn on_commit(mut self, f: impl Fn(&$value, &mut gpui::App) + 'static) -> Self {
            self.state.on_commit(f);
            self
        }
        pub fn on_begin(mut self, f: impl Fn(&mut gpui::App) -> bool + 'static) -> Self {
            self.state.on_begin(f);
            self
        }
        pub fn live(mut self, f: impl Fn(&gpui::App) -> $value + 'static) -> Self {
            self.state.set_live(f);
            self
        }
    };
}
pub(crate) use field_api;
/// Record hit bounds without a layout feedback loop.
pub(crate) fn measure<T: 'static>(
    entity: Entity<T>,
    set: fn(&mut T, Bounds<Pixels>),
) -> impl IntoElement {
    canvas(
        move |bounds, _, cx| {
            entity.update(cx, |this, _| set(this, bounds));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}
/// Install global drag listeners during paint (GPUI requires this phase).
pub(crate) fn drag_listeners<T: 'static>(
    entity: Entity<T>,
    moved: fn(&mut T, &MouseMoveEvent, &mut Window, &mut Context<T>),
    up: fn(&mut T, &mut Window, &mut Context<T>),
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            let weak = entity.downgrade();
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, w, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = weak.update(cx, |this, cx| moved(this, e, w, cx));
                }
            });
            let weak = entity.downgrade();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, w, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = weak.update(cx, |this, cx| up(this, w, cx));
                }
            });
        },
    )
    .absolute()
    .size_full()
}
