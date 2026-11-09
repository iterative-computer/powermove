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
        // The animation plays from its start whenever it comes into view.
        if frame.animate && !self.frame.animate {
            self.started = Instant::now();
        }
        self.frame = frame;
        cx.notify();
    }
    pub fn time(&self) -> f64 {
        self.clock
            .unwrap_or_else(|| self.started.elapsed().as_secs_f64())
    }
    fn act(&self, id: &str, cx: &mut App) {
        (self.on_action)(id, cx);
    }
    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "enter" if !event.is_held => {
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
        let focus = c.focus;
        let mut element = div()
            .id(SharedString::from(format!("welcome-{}", button.id)))
            .debug_selector(|| format!("welcome-{id}", id = id.clone()))
            .role(Role::Button)
            .aria_label(button.label.clone())
            .aria_disabled(!enabled)
            .tab_index(0)
            .focus_visible_style(move |s| s.ring(px(2.)).ring_color(rgb(focus)))
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
    /// A thin bar: filled to its fraction, or a sliding segment while the total
    /// is unknown (still under Reduce Motion).
    fn progress(&self, progress: &Progress, time: Option<f64>) -> impl IntoElement + use<> {
        let t = &self.tokens;
        let (height, radius) = (px(t.metrics.gap / 2.), px(t.metrics.gap / 4.));
        let fill = |w: f32| {
            div()
                .h_full()
                .w(relative(w))
                .rounded(radius)
                .bg(rgb(t.colors.accent))
        };
        let mut bar = div()
            .relative()
            .w_full()
            .h(height)
            .rounded(radius)
            .overflow_hidden()
            .bg(rgb(t.colors.panel_secondary));
        bar = match (progress, time) {
            (Progress::Determinate(fraction), _) => bar.child(fill(fraction.clamp(0., 1.) as f32)),
            (Progress::Indeterminate, Some(time)) => {
                // A quarter-width segment crossing every 1.5 s.
                let at = (time / 1.5).fract() as f32 * 1.25 - 0.25;
                bar.child(fill(0.25).absolute().top_0().left(relative(at)))
            }
            (Progress::Indeterminate, None) => bar,
        };
        bar
    }
    /// The Settings › Project card: one row per step (or folder), title left,
    /// value right, the running step's bar under its row, inset hairlines between.
    fn steps(&self, time: Option<f64>) -> impl IntoElement + use<> {
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let mut card = div()
            .id("welcome-steps")
            .role(Role::List)
            .flex()
            .flex_col()
            .rounded(px(m.group_radius))
            .bg(rgb(c.panel));
        for (index, step) in self.frame.steps.iter().enumerate() {
            let label = if step.value.is_empty() {
                step.title.clone()
            } else {
                format!("{}, {}", step.title, step.value)
            };
            let mut row = div()
                .id(SharedString::from(format!("welcome-step-{index}")))
                .role(Role::ListItem)
                .aria_label(label)
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(m.gap / 2.))
                .min_h(px(m.form_row))
                .px(px(m.inset))
                .py(px(m.gap))
                .text_size(px(t.typography.body))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .gap(px(m.gap))
                        .child(div().text_color(rgb(c.text)).child(step.title.clone()))
                        .child(div().text_color(rgb(c.secondary)).child(step.value.clone())),
                );
            if let Some(progress) = &step.progress {
                row = row.child(self.progress(progress, time));
            }
            if index > 0 {
                card = card.child(div().ml(px(m.inset)).h(px(m.border)).bg(rgb(c.separator)));
            }
            card = card.child(row);
        }
        card
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
                .role(Role::RadioButton)
                .aria_label(choice.label.clone())
                .aria_selected(selected)
                .tab_index(0)
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
        let spin = frame.spinner && !t.accessibility.reduce_motion;
        let sliding = !t.accessibility.reduce_motion
            && frame
                .steps
                .iter()
                .any(|s| s.progress == Some(Progress::Indeterminate));
        if (spin || sliding || time.is_some_and(|time| time < welcome::ANIMATION_SECONDS))
            && self.clock.is_none()
        {
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
                    .id("welcome-title")
                    .debug_selector(|| "welcome-title".into())
                    .role(Role::Heading)
                    .aria_label(frame.title.clone())
                    .text_size(px(t.typography.display))
                    .text_color(rgb(c.text))
                    .child(frame.title.clone()),
            )
            .child(
                div()
                    .id("welcome-line")
                    .role(Role::Label)
                    .aria_label(frame.line.clone())
                    .text_size(px(t.typography.reading))
                    .text_color(rgb(c.secondary))
                    .child(frame.line.clone()),
            );
        if let Some(status) = &frame.status {
            let mut row = div()
                .id("welcome-status")
                .role(Role::Label)
                .aria_label(status.clone())
                .flex()
                .items_center()
                .gap(px(m.gap))
                .debug_selector(|| "welcome-status".into())
                .text_size(px(t.typography.body))
                .text_color(rgb(c.secondary));
            if frame.spinner {
                row = row.child(spinner(if spin { self.time() } else { 0. }, &t));
            }
            content = content.child(row.child(status.clone()));
        }
        if !frame.steps.is_empty() {
            content = content.child(self.steps(sliding.then(|| self.time())));
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
                    .role(Role::DisclosureTriangle)
                    .aria_label("Details")
                    .aria_expanded(self.details_open)
                    .tab_index(0)
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
                        .id("welcome-details-text")
                        .role(Role::Label)
                        .aria_label(details.clone())
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
        // The accessibility tree hangs off an identified, labelled root, as in
        // the editor; without it VoiceOver sees only the traffic lights.
        let mut root = div()
            .id("welcome")
            .role(Role::Group)
            .accessibility_id("shell.welcome")
            .aria_label(frame.title.clone())
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
                    .role(Role::Button)
                    .aria_label(frame.back.clone().unwrap_or_default())
                    .tab_index(0)
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
/// Three quarters of a ring, one turn a second. Still under Reduce Motion.
fn spinner(time: f64, tokens: &Tokens) -> impl IntoElement + use<> {
    let colour = tokens.colors.secondary;
    let size = tokens.typography.reading;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let radius = f64::from(size) / 2. - 1.;
            let centre = bounds.center();
            let at = |turn: f64| {
                let angle = turn * std::f64::consts::TAU;
                centre
                    + point(
                        px((angle.cos() * radius) as f32),
                        px((angle.sin() * radius) as f32),
                    )
            };
            let mut builder = PathBuilder::stroke(px(1.5));
            const SEGMENTS: usize = 24;
            builder.move_to(at(time));
            for segment in 1..=SEGMENTS {
                builder.line_to(at(time + 0.75 * segment as f64 / SEGMENTS as f64));
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, rgb(colour));
            }
        },
    )
    .flex_none()
    .size(px(size))
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
        size(
            px(tokens.metrics.welcome_width),
            px(tokens.metrics.welcome_height),
        ),
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
