//! A window-local, non-focusing action toast shared by Home and the editor.
use gpui::{prelude::*, *};
use pm_design::{Elevation, Text};
use pm_shell::kit::{Button, Kit, Recipes};
use pm_shell::symbol::Weight;
use pm_surface::kit::ActionToast;
use std::time::{Duration, Instant};

pub(crate) struct ToastView {
    toast: Option<ActionToast<()>>,
    icon: Option<&'static str>,
    action: Option<Box<dyn Fn(&mut App)>>,
    action_label: &'static str,
    hovered: bool,
    motion: pm_shell::motion::Motion<u64>,
}
impl ToastView {
    pub(crate) fn new(
        message: String,
        action: impl Fn(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_action(message, "Undo", action, cx)
    }

    pub(crate) fn with_action(
        message: String,
        action_label: &'static str,
        action: impl Fn(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(None, message, Some((action_label, Box::new(action))), cx)
    }

    pub(crate) fn with_icon_action(
        icon: &'static str,
        message: String,
        action_label: &'static str,
        action: impl Fn(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(
            Some(icon),
            message,
            Some((action_label, Box::new(action))),
            cx,
        )
    }

    pub(crate) fn notice(
        icon: Option<&'static str>,
        message: String,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(icon, message, None, cx)
    }

    fn build(
        icon: Option<&'static str>,
        message: String,
        action: Option<(&'static str, Box<dyn Fn(&mut App)>)>,
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
        let (action_label, action) = match action {
            Some((label, cb)) => (label, Some(cb)),
            None => ("", None),
        };
        Self {
            toast: Some(ActionToast::new(message, (), Instant::now())),
            icon,
            action,
            action_label,
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
        let action_btn = self.action.is_some().then(|| {
            kit.button(
                "toast-action",
                self.action_label,
                Button::Neutral,
                None,
                None,
            )
            .on_click(cx.listener(|v, _, _, cx| {
                if v.toast.take().is_some() {
                    if let Some(action) = &v.action {
                        action(cx);
                    }
                    cx.notify();
                }
            }))
        });
        let pill = if let Some(icon) = self.icon {
            let t = &kit.t;
            div()
                .id("action-toast")
                .flex()
                .items_center()
                .gap(px(t.space.md))
                .px(px(t.space.lg))
                .py(px(t.space.md))
                .bg(rgb(t.colors.floating))
                .text_color(rgb(t.colors.text))
                .text_role(t, Text::Label)
                .corner(t, t.radius.lg)
                .elevated(t, Elevation::Menu)
                .aria_label(message.clone())
                .child(kit.symbol(icon, 13., Weight::Medium, t.colors.warning))
                .child(message)
                .children(action_btn)
        } else if let Some(action) = action_btn {
            kit.action_toast(message, action)
        } else {
            let t = &kit.t;
            div()
                .id("action-toast")
                .flex()
                .items_center()
                .gap(px(t.space.md))
                .px(px(t.space.lg))
                .py(px(t.space.md))
                .bg(rgb(t.colors.floating))
                .text_color(rgb(t.colors.text))
                .text_role(t, Text::Label)
                .corner(t, t.radius.lg)
                .elevated(t, Elevation::Menu)
                .aria_label(message.clone())
                .child(message)
        };
        div()
            .absolute()
            .bottom(px(24.))
            .left(px(0.))
            .w_full()
            .flex()
            .justify_center()
            .child(
                pill.role(Role::Status)
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
