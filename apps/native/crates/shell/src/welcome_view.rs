//! The Welcome window: a standard titled window, title hidden, close button only.
//! Closing it quits the app; quitting at any point is safe and the next launch resumes.
use crate::welcome::{self, Frame, Progress, Severity};
use gpui::{prelude::*, *};
use pm_design::Tokens;
use std::{rc::Rc, time::Instant};

pub type OnAction = Rc<dyn Fn(&str, &mut App)>;

pub struct WelcomeView {
    pub tokens: Tokens,
    pub frame: Frame,
    on_action: OnAction,
    focus: FocusHandle,
    started: Instant,
    details_open: bool,
    /// Tests pin the animation clock; the window uses wall time.
    pub clock: Option<f64>,
}
impl WelcomeView {
    pub fn new(tokens: Tokens, frame: Frame, on_action: OnAction, cx: &mut Context<Self>) -> Self {
        Self {
            tokens,
            frame,
            on_action,
            focus: cx.focus_handle(),
            started: Instant::now(),
            details_open: false,
            clock: None,
        }
    }
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }
    pub fn set_frame(&mut self, frame: Frame, cx: &mut Context<Self>) {
        // Each state change restarts nothing but the Details disclosure.
        if frame.surface != self.frame.surface {
            self.details_open = false;
        }
        self.frame = frame;
        cx.notify();
    }
    fn time(&self) -> f64 {
        self.clock
            .unwrap_or_else(|| self.started.elapsed().as_secs_f64())
    }
    fn act(&self, id: &str, cx: &mut App) {
        (self.on_action)(id, cx);
    }
    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "enter" => {
                if let Some(id) = self.frame.default_action().map(str::to_owned) {
                    self.act(&id, cx);
                }
            }
            "left" | "right" => {
                let forward = event.keystroke.key == "right";
                if let Some(id) = self.frame.stepped_choice(forward).map(str::to_owned) {
                    self.act(&format!("choose:{id}"), cx);
                }
            }
            _ => {}
        }
    }
    fn button(&self, button: &welcome::Button, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let primary = self.frame.default.as_deref() == Some(&button.id);
        let enabled = button.enabled;
        let id = button.id.clone();
        let mut element = div()
            .id(SharedString::from(format!("welcome-{}", button.id)))
            .debug_selector(|| format!("welcome-{id}", id = id.clone()))
            .px(px(m.inset))
            .h(px(m.row))
            .flex()
            .items_center()
            .rounded(px(m.control_radius))
            .text_size(px(t.typography.body))
            .child(button.label.clone());
        element = if primary {
            element.bg(rgb(c.accent)).text_color(rgb(c.selected_text))
        } else {
            element.bg(rgb(c.panel_secondary)).text_color(rgb(c.text))
        };
        if !enabled {
            element = element.opacity(0.4);
        } else {
            let id = button.id.clone();
            element = element.on_click(cx.listener(move |v, _, _, cx| v.act(&id, cx)));
        }
        element
    }
    fn progress(&self, progress: &Progress) -> impl IntoElement + use<> {
        let t = &self.tokens;
        let fraction = match progress {
            Progress::Determinate { done, total } => *done as f32 / (*total).max(1) as f32,
            Progress::Indeterminate => 0.,
        };
        let mut bar = div()
            .w_full()
            .h(px(t.metrics.gap / 2.))
            .rounded(px(t.metrics.gap / 4.))
            .bg(rgb(t.colors.panel_secondary));
        if matches!(progress, Progress::Determinate { .. }) {
            bar = bar.child(
                div()
                    .h_full()
                    .w(relative(fraction.clamp(0., 1.)))
                    .rounded(px(t.metrics.gap / 4.))
                    .bg(rgb(t.colors.accent)),
            );
        }
        bar
    }
    fn choices(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let mut group = if self.frame.cards {
            div().flex().flex_row().gap(px(m.inset))
        } else {
            div()
                .flex()
                .flex_col()
                .rounded(px(m.group_radius))
                .bg(rgb(c.panel))
        };
        for choice in &self.frame.choices {
            let selected = self.frame.chosen.as_ref() == Some(&choice.id);
            let id = choice.id.clone();
            let mut item = div()
                .id(SharedString::from(format!("choice-{}", choice.id)))
                .debug_selector(|| format!("choice-{id}", id = id.clone()))
                .flex()
                .flex_col()
                .gap(px(m.gap / 2.))
                .p(px(m.inset))
                .on_click({
                    let id = choice.id.clone();
                    cx.listener(move |v, _, _, cx| v.act(&format!("choose:{id}"), cx))
                });
            if self.frame.cards {
                item = item
                    .flex_1()
                    .rounded(px(m.group_radius))
                    .bg(rgb(c.panel))
                    .border_2()
                    .border_color(rgb(if selected { c.accent } else { c.panel }));
                for line in &choice.lines {
                    item = item.child(
                        div()
                            .h(px(m.property_row))
                            .px(px(m.gap))
                            .flex()
                            .items_center()
                            .rounded(px(m.control_radius))
                            .bg(rgb(c.panel_secondary))
                            .text_size(px(t.typography.body))
                            .text_color(rgb(c.secondary))
                            .child(line.clone()),
                    );
                }
                item = item.child(
                    div()
                        .text_size(px(t.typography.title))
                        .text_color(rgb(c.text))
                        .child(choice.label.clone()),
                );
            } else {
                if selected {
                    item = item.bg(rgb(c.accent_wash));
                }
                item = item.child(
                    div()
                        .text_size(px(t.typography.body))
                        .text_color(rgb(c.text))
                        .child(choice.label.clone()),
                );
                if let Some(detail) = &choice.detail {
                    item = item.child(
                        div()
                            .text_size(px(t.typography.body))
                            .text_color(rgb(c.secondary))
                            .child(detail.clone()),
                    );
                }
            }
            group = group.child(item);
        }
        group
    }
}
impl Render for WelcomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.tokens.clone();
        let (c, m) = (&t.colors, &t.metrics);
        let frame = self.frame.clone();
        let time = (frame.animate && !t.accessibility.reduce_motion).then(|| self.time());
        if time.is_some_and(|time| time < welcome::ANIMATION_SECONDS) && self.clock.is_none() {
            window.request_animation_frame();
        }
        let mut content = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(m.gap))
            .w_full()
            .max_w(px(m.sheet_width_wide));
        if let Some(label) = &frame.eyebrow {
            content = content.child(
                div()
                    .text_size(px(t.typography.body))
                    .text_color(rgb(c.secondary))
                    .child(label.clone()),
            );
        }
        content = content
            .child(
                div()
                    .debug_selector(|| "welcome-title".into())
                    .text_size(px(t.typography.display))
                    .text_color(rgb(c.text))
                    .child(frame.title.clone()),
            )
            .child(
                div()
                    .text_size(px(t.typography.reading))
                    .text_color(rgb(c.secondary))
                    .child(frame.line.clone()),
            );
        if let Some(status) = &frame.status {
            content = content.child(
                div()
                    .debug_selector(|| "welcome-status".into())
                    .text_size(px(t.typography.body))
                    .text_color(rgb(c.secondary))
                    .child(status.clone()),
            );
        }
        if let Some(progress) = &frame.progress {
            content = content.child(self.progress(progress));
        }
        if !frame.choices.is_empty() {
            content = content.child(self.choices(cx));
        }
        if !frame.rows.is_empty() {
            let mut card = div()
                .flex()
                .flex_col()
                .rounded(px(m.group_radius))
                .bg(rgb(c.panel));
            for row in &frame.rows {
                let tone = match row.severity {
                    Severity::Warning => c.warning,
                    Severity::Danger => c.danger,
                    Severity::Note => c.secondary,
                };
                card = card.child(
                    div()
                        .flex()
                        .gap(px(m.gap))
                        .p(px(m.inset))
                        .text_size(px(t.typography.body))
                        .child(div().flex_none().child(crate::symbol::symbol(
                            row.symbol,
                            t.typography.body,
                            crate::symbol::Weight::Medium,
                            tone,
                            window,
                        )))
                        .child(div().text_color(rgb(c.text)).child(row.text.clone())),
                );
            }
            content = content.child(card);
        }
        if let Some(error) = &frame.error {
            content = content.child(
                div()
                    .text_size(px(t.typography.body))
                    .text_color(rgb(c.danger))
                    .child(error.clone()),
            );
        }
        if let Some(note) = &frame.note {
            content = content.child(
                div()
                    .text_size(px(t.typography.body))
                    .text_color(rgb(c.secondary))
                    .child(note.clone()),
            );
        }
        if let Some(details) = &frame.details {
            content = content.child(
                div()
                    .id("welcome-details")
                    .debug_selector(|| "welcome-details".into())
                    .text_size(px(t.typography.body))
                    .text_color(rgb(c.secondary))
                    .child("Details")
                    .on_click(cx.listener(|v, _, _, cx| {
                        v.details_open = !v.details_open;
                        cx.notify();
                    })),
            );
            if self.details_open {
                content = content.child(
                    div()
                        .font_family(pm_design::MONO_FONT)
                        .text_size(px(t.typography.body))
                        .text_color(rgb(c.text))
                        .child(details.clone()),
                );
            }
        }
        let mut footer = div().flex().w_full().gap(px(m.gap)).pt(px(m.inset));
        for button in frame.buttons.iter().filter(|b| b.leading) {
            footer = footer.child(self.button(button, cx));
        }
        footer = footer.child(div().flex_1());
        for button in frame.buttons.iter().filter(|b| !b.leading) {
            footer = footer.child(self.button(button, cx));
        }
        content = content.child(footer);
        let mut root = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p(px(m.sheet_inset))
            .bg(rgb(c.window))
            .font_family(pm_design::TEXT_FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key));
        if let Some(time) = time {
            root = root.child(mark_canvas(time, &t));
        }
        if frame.back.is_some() {
            root = root.child(
                div()
                    .id("welcome-back")
                    .debug_selector(|| "welcome-back".into())
                    .absolute()
                    .top(px(m.toolbar))
                    .left(px(m.sheet_inset))
                    .text_size(px(t.typography.body))
                    .text_color(rgb(c.text))
                    .flex()
                    .items_center()
                    .gap(px(m.gap / 2.))
                    .child(crate::symbol::symbol(
                        "chevron.left",
                        t.typography.body,
                        crate::symbol::Weight::Medium,
                        c.text,
                        window,
                    ))
                    .child("Back")
                    .on_click(cx.listener(|v, _, _, cx| v.act("back", cx))),
            );
        }
        root.child(content)
    }
}
/// The first-run mark, drawn on the window's own ground. Its bloom is stacked
/// translucent strokes; the mark itself stays hollow, as in the original scene.
fn mark_canvas(time: f64, tokens: &Tokens) -> impl IntoElement + use<> {
    let accent = tokens.colors.accent;
    let ground = tokens.colors.window;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let Some(state) = welcome::mark_at(time) else {
                return;
            };
            let size = [f64::from(bounds.size.width), f64::from(bounds.size.height)];
            let (fit, origin) = welcome::fit_stage(size);
            let point_at = |x: f64, y: f64| {
                point(
                    bounds.origin.x
                        + px(((state.center[0] + x * state.scale) * fit + origin[0]) as f32),
                    bounds.origin.y
                        + px(((state.center[1] + y * state.scale) * fit + origin[1]) as f32),
                )
            };
            let outline = |mut builder: PathBuilder| {
                let mark = &welcome::MARK;
                builder.move_to(point_at(mark[0][0], mark[0][1]));
                for i in 0..mark.len() {
                    let (a, b) = (mark[i], mark[(i + 1) % mark.len()]);
                    builder.cubic_bezier_to(
                        point_at(b[0], b[1]),
                        point_at(a[0] + a[4], a[1] + a[5]),
                        point_at(b[0] + b[2], b[1] + b[3]),
                    );
                }
                builder.close();
                builder.build()
            };
            let mask = ContentMask {
                bounds,
                fade_out: Default::default(),
            };
            window.with_content_mask(Some(mask), |window| {
                const STEPS: usize = 14;
                if state.glow > 0. {
                    for step in (1..=STEPS).rev() {
                        let spread = 72.876 * fit * step as f64 / STEPS as f64;
                        if let Ok(path) = outline(PathBuilder::stroke(px(spread as f32 * 2.))) {
                            let alpha = state.glow / STEPS as f64;
                            let mut colour = Rgba::from(rgb(accent));
                            colour.alpha = alpha as f32;
                            window.paint_path(path, colour);
                        }
                    }
                }
                if let Ok(path) = outline(PathBuilder::fill()) {
                    window.paint_path(path, rgb(ground));
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
/// Standard titled window, centred on the active display. Closing it quits.
pub fn open_welcome(
    tokens: Tokens,
    frame: Frame,
    on_action: OnAction,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<WelcomeView>> {
    let bounds = Bounds::centered(
        None,
        size(px(tokens.metrics.welcome_width), px(tokens.metrics.welcome_height)),
        cx,
    );
    cx.open_window(
        WindowOptions::new()
            .window_bounds(Some(WindowBounds::Windowed(bounds)))
            .titlebar(Some(TitlebarOptions {
                title: Some("Welcome to Powermove".into()),
                appears_transparent: true,
                ..Default::default()
            }))
            .is_resizable(false)
            .is_minimizable(false),
        |window, cx| {
            let view = cx.new(|cx| WelcomeView::new(tokens, frame, on_action, cx));
            window.focus(&view.read(cx).focus_handle(), cx);
            window.on_window_should_close(cx, |_, cx| {
                cx.quit();
                true
            });
            view.update(cx, |_, cx| {
                cx.observe_window_appearance(window, |v, _, cx| {
                    v.tokens = crate::macos::tokens();
                    cx.notify();
                })
                .detach();
            });
            view
        },
    )
}
