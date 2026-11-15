//! A window-local, non-focusing action toast shared by Home and the editor.
use gpui::{prelude::*, *};
use pm_shell::kit::{Button, Kit};
use pm_surface::kit::ActionToast;
use std::time::{Duration, Instant};

pub(crate) struct ToastView {
    toast: Option<ActionToast<()>>,
    action: Box<dyn Fn(&mut App)>,
    hovered: bool,
    motion: pm_shell::motion::Motion<u64>,
}
impl ToastView {
    pub(crate) fn new(
        message: String,
        action: impl Fn(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let alive = this.update(cx, |v, cx| {
                    if v.toast
                        .as_mut()
                        .is_some_and(|t| t.update(Instant::now(), v.hovered))
                    {
                        v.toast = None;
                        cx.notify();
                    }
                    v.toast.is_some()
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        Self {
            toast: Some(ActionToast::new(message, (), Instant::now())),
            action: Box::new(action),
            hovered: false,
            motion: Default::default(),
        }
    }
}
impl Render for ToastView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(toast) = &self.toast else {
            return div().into_any_element();
        };
        let message = toast.message.clone();
        let tokens = pm_shell::tokens();
        let kit = Kit::new(&tokens, &self.motion, |v| &mut v.motion, window, cx);
        let undo = kit
            .button("delete-undo", "Undo", Button::Neutral, None, None)
            .on_click(cx.listener(|v, _, _, cx| {
                if v.toast.take().is_some() {
                    (v.action)(cx);
                    cx.notify();
                }
            }));
        div()
            .absolute()
            .bottom(px(24.))
            .left(px(0.))
            .w_full()
            .flex()
            .justify_center()
            .child(
                kit.action_toast(message, undo)
                    .role(Role::Status)
                    .on_hover(cx.listener(|v, hovered: &bool, _, cx| {
                        if let Some(toast) = &mut v.toast {
                            toast.update(Instant::now(), *hovered);
                        }
                        v.hovered = *hovered;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}
