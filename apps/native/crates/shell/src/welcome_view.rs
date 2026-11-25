//! The Welcome window: a standard titled window, title hidden, close button only.
//! Closing it quits the app; quitting at any point is safe and the next launch resumes.
use crate::welcome::{self, Frame, Progress, Severity};
use crate::{
    kit::{Button, Kit, Recipes},
    motion::Motion,
    native_control::{ControlSpec, Kind, NativeControl},
};
use gpui::{prelude::*, *};
use pm_design::{Elevation, Tokens};
use std::{rc::Rc, time::Instant};

pub type OnAction = Rc<dyn Fn(&str, &mut App)>;

pub struct WelcomeView {
    motion: Motion<u64>,
    pub tokens: Tokens,
    pub frame: Frame,
    on_action: OnAction,
    focus: FocusHandle,
    opened: Instant,
    /// When the animation last started, in seconds since the view opened.
    started: f64,
    details_open: bool,
    /// Show All under a list of names.
    names_open: bool,
    /// The rows and names that scroll, and how far they could, as last laid out.
    body_scroll: ScrollHandle,
    body_extent: Option<Pixels>,
    /// Tests pin the clock (seconds since the view opened); the window uses wall time.
    pub clock: Option<f64>,
    /// The card on the first-run stage: sized to its content on the
    /// stage's room, its buttons AppKit's.
    pub stage: bool,
    /// The stage card's AppKit buttons, by id and label.
    natives: std::collections::HashMap<(String, String), Entity<NativeControl>>,
    /// While a wave carries the card: the snapshot its buttons paint.
    snapshot: Option<u64>,
    snapshots: u64,
}
impl WelcomeView {
    fn motion(&mut self) -> &mut Motion<u64> {
        &mut self.motion
    }
    pub fn new(tokens: Tokens, frame: Frame, on_action: OnAction, cx: &mut Context<Self>) -> Self {
        Self {
            motion: Motion::default(),
            tokens,
            frame,
            on_action,
            focus: cx.focus_handle(),
            opened: Instant::now(),
            started: 0.,
            details_open: false,
            names_open: false,
            body_scroll: ScrollHandle::new(),
            body_extent: None,
            clock: None,
            stage: false,
            natives: Default::default(),
            snapshot: None,
            snapshots: 0,
        }
    }
    /// A wave is carrying the card (`true`): its AppKit buttons paint their
    /// own pictures in GPUI, so the wave distorts them with the card, and
    /// come back live when it ends.
    pub fn set_snapshot(&mut self, on: bool, cx: &mut Context<Self>) {
        if on == self.snapshot.is_some() {
            return;
        }
        self.snapshot = on.then(|| {
            self.snapshots += 1;
            self.snapshots
        });
        let build = self.snapshot.map(crate::stage::snapshot_build);
        for control in self.natives.values() {
            control.update(cx, |c, cx| {
                if c.set_build_snapshot(build) {
                    cx.notify();
                }
            });
        }
        cx.notify();
    }
    /// The stage card's AppKit button for `button`, made once per label.
    fn native_button(
        &mut self,
        button: &welcome::Button,
        primary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<NativeControl> {
        let key = (button.id.clone(), button.label.clone());
        if let Some(control) = self.natives.get(&key) {
            let disabled = !button.enabled;
            control.update(cx, |c, cx| {
                c.update(&serde_json::Value::Null, &[], disabled, cx)
            });
            return control.clone();
        }
        let tokens = self.tokens.clone();
        let identifier = format!("welcome-{}", button.id);
        let control = cx.new(|cx| {
            let mut control = NativeControl::new(
                ControlSpec {
                    kind: Kind::Push,
                    label: &button.label,
                    identifier: &identifier,
                    value: &serde_json::Value::Null,
                    options: vec![],
                    disabled: !button.enabled,
                    default: primary,
                    destructive: false,
                    inline: false,
                    quiet: false,
                    pull: false,
                    tint: None,
                    symbol: None,
                    glass: primary,
                    well: false,
                },
                &tokens,
                window,
                cx,
            );
            control.set_build_snapshot(self.snapshot.map(crate::stage::snapshot_build));
            if primary && self.stage {
                control.prominent(PRIMARY_WIDTH);
            } else if self.stage {
                control.extra_large();
            }
            control
        });
        let id = button.id.clone();
        cx.subscribe(
            &control,
            move |v: &mut Self, _, _: &serde_json::Value, cx| v.act(&id, cx),
        )
        .detach();
        self.natives.insert(key, control.clone());
        control
    }
    /// Gives the view the keys, and takes them back when the control that had
    /// them leaves with its step (Begin, Back): GPUI then leaves the window
    /// with no focus, and Return and the arrows reach nothing.
    pub fn hold_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        cx.on_focus_lost(window, |view, window, cx| window.focus(&view.focus, cx))
            .detach();
    }
    pub fn set_frame(&mut self, frame: Frame, cx: &mut Context<Self>) {
        if frame.surface != self.frame.surface {
            self.details_open = false;
            self.names_open = false;
            self.body_scroll = ScrollHandle::new();
            self.body_extent = None;
        }
        if frame.animate && !self.frame.animate {
            self.started = self.now();
        }
        self.frame = frame;
        cx.notify();
    }
    fn now(&self) -> f64 {
        self.clock
            .unwrap_or_else(|| self.opened.elapsed().as_secs_f64())
    }
    /// Seconds into the animation.
    pub fn time(&self) -> f64 {
        self.now() - self.started
    }
    /// Runs the action after this window's own update: the action may update
    /// or close the window, which GPUI refuses from inside its listeners.
    fn act(&self, id: &str, cx: &mut App) {
        let (on_action, id) = (self.on_action.clone(), id.to_owned());
        cx.defer(move |cx| on_action(&id, cx));
    }
    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "enter" if !event.is_held => {
                if let Some(id) = self.frame.default_action().map(str::to_owned) {
                    // Return is the default button's alone: a button that has
                    // the keys (the one last clicked) would press itself too
                    // when the key comes up.
                    window.focus(&self.focus, cx);
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
    fn button(
        &mut self,
        button: &welcome::Button,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let primary = self.frame.default.as_deref() == Some(&button.id);
        if self.stage && crate::text::available(window) {
            return self
                .native_button(button, primary, window, cx)
                .into_any_element();
        }
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let enabled = button.enabled;
        let id = button.id.clone();
        let mut element = div()
            .id(SharedString::from(format!("welcome-{}", button.id)))
            .debug_selector(|| format!("welcome-{id}", id = id.clone()))
            .role(Role::Button)
            .aria_label(button.label.clone())
            .aria_disabled(!enabled)
            .tab_index(0)
            .flex()
            .items_center()
            .justify_center()
            .child(button.label.clone());
        // Headless (no AppKit): the stage's large glass button, drawn.
        element = if self.stage && primary {
            // The extra-large glass button (`NativeControl::prominent`).
            element
                .w(px(PRIMARY_WIDTH))
                .h(px(PRIMARY_HEIGHT))
                .rounded_full()
                .text_size(px(t.typography.heading))
                .font_weight(FontWeight::MEDIUM)
        } else if self.stage {
            // Beside it, the same size.
            element
                .px(px(18.))
                .h(px(PRIMARY_HEIGHT))
                .rounded_full()
                .text_size(px(t.typography.heading))
        } else {
            element
                .px(px(m.inset))
                .h(px(m.row))
                .rounded(px(m.control_radius))
                .text_size(px(t.typography.body))
        };
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
        element.into_any_element()
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
    fn steps(&self, kit: &Kit, time: Option<f64>) -> impl IntoElement + use<> {
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
                card = card.child(kit.separator().ml(px(m.inset)));
            }
            card = card.child(row);
        }
        card
    }
    /// Counts side by side in one card: the figure large, what it counts under it.
    fn stats(&self, kit: &Kit) -> impl IntoElement + use<> {
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let mut card = div()
            .id("welcome-stats")
            .debug_selector(|| "welcome-stats".into())
            .role(Role::List)
            .flex()
            .flex_row()
            .rounded(px(m.group_radius))
            .bg(rgb(c.panel));
        for (index, stat) in self.frame.stats.iter().enumerate() {
            if index > 0 {
                card = card.child(kit.vertical_separator().my(px(t.space.lg)));
            }
            card = card.child(
                div()
                    .id(SharedString::from(format!("welcome-stat-{index}")))
                    .role(Role::ListItem)
                    .aria_label(format!("{} {}", stat.value, stat.label))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.xs))
                    .px(px(t.space.xl))
                    .py(px(t.space.lg))
                    .child(
                        div()
                            .text_size(px(t.typography.display))
                            .text_color(rgb(c.text))
                            .child(stat.value.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(t.typography.body))
                            .text_color(rgb(c.secondary))
                            .child(stat.label.clone()),
                    ),
            );
        }
        card
    }
    /// The stage's window in two columns, as Arc's Create Account window
    /// is built: on the left the information (the step's title and line,
    /// the steps and how far along they are, Powermove's mark and name);
    /// on the right, in a panel of its own, the action (the step's choices
    /// or progress, its one primary action on the panel's bottom line). On
    /// the Welcome the panel holds the lit mark (`stage::WINDOW_MARK_X`).
    fn columns(&self, info: Div, action: Div, window: &mut Window) -> Div {
        let t = &self.tokens;
        let c = &t.colors;
        let hairline = rgba((c.text << 8) | 0x12);
        let left = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .w(px(LEFT_COLUMN))
            .flex()
            .flex_col()
            .pt(px(COLUMN_TOP))
            .pb(px(PRIMARY_BOTTOM + 12.))
            .px(px(COLUMN_INSET))
            .child(info)
            .child(div().flex_1())
            .child(self.identity(window));
        let right = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .left(px(LEFT_COLUMN))
            .bg(rgba((c.text << 8) | 0x08))
            .border_l_1()
            .border_color(hairline)
            .flex()
            .flex_col()
            .pt(px(COLUMN_TOP))
            .pb(px(PRIMARY_BOTTOM))
            .px(px(PANEL_INSET))
            .child(action);
        div().relative().size_full().child(left).child(right)
    }
    /// The steps as a list down the left column: a numeral in a ring for
    /// each step to come, the step under way in the accent, a check for one
    /// done. On the Welcome (none under way) each step has its line.
    fn steps_list(&self, outline: &welcome::Outline, window: &mut Window) -> Stateful<Div> {
        let t = &self.tokens;
        let c = &t.colors;
        let welcome = outline.current.is_none();
        let mut list = div()
            .id("welcome-outline")
            .debug_selector(|| "welcome-outline".into())
            .role(Role::List)
            .aria_label(format!("Steps: {}", outline.steps.join(", ")))
            .flex()
            .flex_col()
            .gap(px(if welcome { 16. } else { 14. }));
        for (index, step) in outline.steps.iter().enumerate() {
            let current = outline.current == Some(index);
            let done = outline.current.is_some_and(|at| index < at);
            let marker = div()
                .flex_none()
                .size(px(20.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD);
            let marker = if current {
                marker
                    .bg(rgb(c.accent))
                    .text_color(rgb(c.selected_text))
                    .child(format!("{}", index + 1))
            } else if done {
                marker
                    .border_1()
                    .border_color(rgba((c.accent << 8) | 0xa0))
                    .child(crate::symbol::symbol(
                        "checkmark",
                        9.,
                        crate::symbol::Weight::Semibold,
                        c.accent,
                        window,
                    ))
            } else {
                marker
                    .border_1()
                    .border_color(rgba((c.secondary << 8) | 0x80))
                    .text_color(rgb(c.secondary))
                    .child(format!("{}", index + 1))
            };
            let note = outline.notes.get(index).filter(|_| welcome).cloned();
            list = list.child(
                div()
                    .id(SharedString::from(format!("welcome-outline-{index}")))
                    .role(Role::ListItem)
                    .aria_label(match &note {
                        Some(note) => format!("{step}: {note}"),
                        None if current => format!("{step}, now"),
                        None if done => format!("{step}, done"),
                        None => step.clone(),
                    })
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(marker)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .pt(px(1.))
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .line_height(px(18.))
                                    .font_weight(if current || welcome {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(if current || welcome {
                                        rgb(c.text)
                                    } else {
                                        rgb(c.secondary)
                                    })
                                    .child(step.clone()),
                            )
                            .children(note.map(|note| {
                                div()
                                    .text_size(px(12.5))
                                    .line_height(relative(1.4))
                                    .text_color(rgb(c.secondary))
                                    .child(note)
                            })),
                    ),
            );
        }
        list
    }
    /// Powermove's mark and name at the foot of the left column.
    fn identity(&self, _: &mut Window) -> Div {
        let t = &self.tokens;
        let c = &t.colors;
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(c.secondary))
            .child(mark_glyph(13., c.secondary))
            .child("Powermove")
    }
    /// A group's heading, as Settings heads its groups.
    fn section_title(&self, title: &str) -> impl IntoElement + use<> {
        let t = &self.tokens;
        div()
            .id(SharedString::from(format!("welcome-heading-{title}")))
            .role(Role::Heading)
            .aria_label(title.to_owned())
            .px(px(t.metrics.inset))
            .text_size(px(t.typography.body))
            .text_color(rgb(t.colors.secondary))
            .child(title.to_owned())
    }
    /// Notes with a symbol: the symbol centred on the first line, the text
    /// wrapping beside it, an optional reason under it. Air, not hairlines,
    /// between rows.
    fn rows(&self, window: &mut Window) -> impl IntoElement + use<> {
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let line = t.typography.body * 1.5;
        let mut card = div()
            .id("welcome-rows")
            .debug_selector(|| "welcome-rows".into())
            .role(Role::List)
            .flex()
            .flex_col()
            .py(px(t.space.sm))
            .rounded(px(m.group_radius))
            .bg(rgb(c.panel));
        for (index, row) in self.frame.rows.iter().enumerate() {
            let tone = match row.severity {
                Severity::Warning => c.warning,
                Severity::Danger => c.danger,
                Severity::Note => c.secondary,
                Severity::Success => c.success,
            };
            let mut text = div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(t.space.hair.max(1.)))
                .child(
                    div()
                        .line_height(px(line))
                        .text_color(rgb(c.text))
                        .child(row.text.clone()),
                );
            if let Some(detail) = &row.detail {
                text = text.child(
                    div()
                        .line_height(px(line))
                        .text_color(rgb(c.secondary))
                        .child(detail.clone()),
                );
            }
            let label = match &row.detail {
                Some(detail) => format!("{}. {detail}", row.text),
                None => row.text.clone(),
            };
            card = card.child(
                div()
                    .id(SharedString::from(format!("welcome-row-{index}")))
                    .debug_selector(move || format!("welcome-row-{index}"))
                    .role(Role::ListItem)
                    .aria_label(label)
                    .flex()
                    .flex_none()
                    .items_start()
                    .gap(px(t.space.md + t.space.xs))
                    .px(px(m.inset))
                    .py(px(t.space.sm))
                    .text_size(px(t.typography.body))
                    .child(div().flex_none().h(px(line)).flex().items_center().child(
                        crate::symbol::symbol(
                            row.symbol,
                            t.typography.body,
                            crate::symbol::Weight::Medium,
                            tone,
                            window,
                        ),
                    ))
                    .child(text),
            );
        }
        card
    }
    /// Names in two columns, read down then across, under their heading; the
    /// first few, then Show All.
    fn names(
        &self,
        kit: &Kit,
        names: &welcome::Names,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let open = self.names_open || names.names.len() <= names.preview;
        let shown: Vec<String> = if open {
            names.names.clone()
        } else {
            names.names.iter().take(names.preview).cloned().collect()
        };
        let half = shown.len().div_ceil(2);
        let column = |part: &[String]| {
            let mut column = div().flex_1().min_w(px(0.)).flex().flex_col();
            for name in part {
                column = column.child(
                    div()
                        .id(SharedString::from(format!("welcome-name-{name}")))
                        .role(Role::ListItem)
                        .aria_label(name.clone())
                        .h(px(t.space.xxl + t.space.xs))
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_size(px(t.typography.body))
                        .text_color(rgb(c.text))
                        .child(name.clone()),
                );
            }
            column
        };
        let mut card = div()
            .id("welcome-names")
            .debug_selector(|| "welcome-names".into())
            .role(Role::List)
            .aria_label(names.title.clone())
            .flex()
            .flex_col()
            .px(px(m.inset))
            .py(px(t.space.sm))
            .rounded(px(m.group_radius))
            .bg(rgb(c.panel))
            .child(
                div()
                    .flex()
                    .gap(px(t.space.xl))
                    .child(column(&shown[..half]))
                    .child(column(&shown[half..])),
            );
        if names.names.len() > names.preview {
            let label = if self.names_open {
                "Show Less".to_owned()
            } else {
                format!("Show All {}", names.names.len())
            };
            // No focus ring, as on every button (owner decision, 2026-11-14).
            card = card.child(
                kit.button(
                    "welcome-names-toggle",
                    label,
                    Button::Quiet,
                    None,
                    Some(if self.names_open {
                        "chevron.up"
                    } else {
                        "chevron.down"
                    }),
                )
                .debug_selector(|| "welcome-names-toggle".into())
                .aria_expanded(self.names_open)
                .on_click(cx.listener(|v, _, _, cx| {
                    v.names_open = !v.names_open;
                    cx.notify();
                })),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(m.gap + 2.))
            .child(self.section_title(&names.title))
            .child(card)
    }
    fn choices(&self, kit: &Kit, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
                    .relative()
                    .pt(px(m.inset + m.symbol_panel))
                    .bg(rgb(if selected { c.raised } else { c.panel }))
                    .when(selected, |d| {
                        d.elevated(t, Elevation::Raised).child(
                            div()
                                .absolute()
                                .top(px(t.space.md))
                                .right(px(t.space.md))
                                .child(kit.symbol(
                                    "checkmark.circle.fill",
                                    m.symbol_panel,
                                    crate::symbol::Weight::Regular,
                                    c.secondary,
                                )),
                        )
                    });
                for line in &choice.lines {
                    item = item.child(
                        div()
                            .h(px(m.property_row))
                            .px(px(m.gap))
                            .flex()
                            .items_center()
                            .rounded(px((m.group_radius - m.inset).max(t.radius.xs)))
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
                    item = item.bg(rgb(c.raised)).elevated(t, Elevation::Raised);
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
        self.motion.frame(window);
        let kit = Kit::new(&self.tokens, &self.motion, Self::motion, window, cx);
        let t = self.tokens.clone();
        let (c, m) = (&t.colors, &t.metrics);
        let frame = self.frame.clone();
        // On the stage the film is the animation.
        let time =
            (frame.animate && !self.stage && !t.accessibility.reduce_motion).then(|| self.time());
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
        let spoken = frame.title.replace('\n', " ");
        // The stage's window: information in the left column, the action in
        // the right (Arc's Create Account window's structure).
        let two = self.stage;
        // A step's screens (`top`) keep their title at one height and their
        // buttons at the bottom; the others centre as one block.
        let air = px(t.space.xxl);
        let mut content = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(m.gap))
            .w_full()
            .when(!two, |d| {
                d.max_w(px(m.sheet_width_wide))
                    .max_h(relative(1.))
                    .min_h(px(0.))
            })
            .when(frame.top && !two, |d| d.h_full().gap(px(0.)))
            .when(two, |d| d.gap(px(0.)))
            .when(frame.centred && !two, |d| d.items_center().text_center());
        // On the stage: the step's action, in the right column's panel.
        let mut action = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(m.gap + 4.))
            .w_full()
            .flex_1()
            .min_h(px(0.));
        macro_rules! act {
            ($e:expr $(,)?) => {{
                let e = $e;
                if two {
                    action = action.child(e);
                } else {
                    content = content.child(e);
                }
            }};
        }
        let mut header = div()
            .flex()
            .flex_col()
            .gap(px(if frame.top {
                m.gap + 2.
            } else if self.stage {
                // The Welcome's title-to-line step.
                10.
            } else {
                m.gap
            }))
            .when(frame.centred && !two, |d| d.items_center());
        if let Some(label) = &frame.eyebrow {
            header = header.child(
                div()
                    .text_size(px(t.typography.body))
                    .text_color(rgb(c.secondary))
                    .child(label.clone()),
            );
        }
        header = header
            .child(
                div()
                    .id("welcome-title")
                    .debug_selector(|| "welcome-title".into())
                    .role(Role::Heading)
                    .aria_label(spoken.clone())
                    .when_else(
                        frame.sheet_title,
                        |d| {
                            d.text_size(px(t.typography.heading))
                                .font_weight(FontWeight(t.typography.display_weight))
                        },
                        |d| {
                            // On the stage the welcome is a title card.
                            // On the stage the window is larger than the
                            // Welcome window: its type steps up with it.
                            let size = if self.stage {
                                STEP_TITLE
                            } else {
                                t.typography.display
                            };
                            d.text_size(px(size)).when(self.stage, |d| {
                                d.font_weight(FontWeight::BOLD)
                                    .line_height(relative(1.15))
                                    .letter_spacing(px(-size * 0.012))
                            })
                        },
                    )
                    .text_color(rgb(c.text))
                    .child(frame.title.clone()),
            )
            .child(
                div()
                    .id("welcome-line")
                    .role(Role::Label)
                    .aria_label(frame.line.clone())
                    .text_size(px(if self.stage {
                        LINE_SIZE
                    } else {
                        t.typography.reading
                    }))
                    .line_height(relative(1.45))
                    .text_color(rgb(c.secondary))
                    .child(frame.line.clone()),
            );
        content = content.child(header);
        if let Some(outline) = &frame.outline {
            content = content.child(div().mt(px(32.)).child(self.steps_list(outline, window)));
        }
        let spaced = |d: Div| if frame.top { d.mt(air) } else { d };
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
            // Progress belongs to the action: on the stage, in the panel.
            act!(spaced(div()).child(row.child(status.clone())));
        }
        if !frame.stats.is_empty() {
            act!(spaced(div().flex_none()).child(self.stats(&kit)));
        }
        if !frame.steps.is_empty() {
            act!(spaced(div().flex_none()).child(self.steps(&kit, sliding.then(|| self.time()))));
        }
        if !frame.choices.is_empty() {
            act!(self.choices(&kit, cx));
        }
        // The rows and the names scroll together in the room left between the
        // header and the buttons; a fade at the bottom says there's more.
        if !frame.rows.is_empty() || frame.names.is_some() {
            let mut body = div()
                .id("welcome-body")
                .debug_selector(|| "welcome-body".into())
                .flex()
                .flex_col()
                .gap(px(if frame.top { t.space.xxl } else { m.gap }))
                .w_full()
                .flex_shrink_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .track_scroll(&self.body_scroll)
                .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()));
            if !frame.rows.is_empty() {
                let mut section = div().flex().flex_col().gap(px(m.gap + 2.));
                if let Some(title) = &frame.rows_title {
                    section = section.child(self.section_title(title));
                }
                body = body.child(section.child(self.rows(window)));
            }
            if let Some(names) = &frame.names {
                body = body.child(self.names(&kit, names, cx));
            }
            // More below the fold: measured on the last layout, so ask for
            // one more frame whenever the measure moves.
            let max = self.body_scroll.max_offset().y;
            if self.body_extent != Some(max) {
                self.body_extent = Some(max);
                window.request_animation_frame();
            }
            let more = max > px(1.) && -self.body_scroll.offset().y < max - px(1.);
            let window_colour = rgba((c.window << 8) | 0xff);
            let clear = rgba(c.window << 8);
            let mut wrapper = div()
                .relative()
                .flex()
                .flex_col()
                .flex_shrink_1()
                .when(frame.top, |d| d.flex_1())
                .min_h(px(0.))
                .when(frame.top, |d| d.mt(air))
                .child(body);
            if more {
                wrapper = wrapper.child(
                    div()
                        .debug_selector(|| "welcome-more".into())
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(m.form_row))
                        .bg(linear_gradient(
                            180.,
                            linear_color_stop(clear, 0.),
                            linear_color_stop(window_colour, 1.),
                        )),
                );
            }
            act!(wrapper);
        } else if frame.top && !two {
            content = content.child(div().flex_1());
        }
        if let Some(error) = &frame.error {
            act!(
                div()
                    .text_size(px(t.typography.body))
                    .text_color(rgb(c.danger))
                    .child(error.clone()),
            );
        }
        if let Some(note) = &frame.note {
            act!(
                div()
                    .when(frame.top, |d| d.mt(px(t.space.xl)))
                    .text_size(px(t.typography.body))
                    .line_height(relative(1.45))
                    .text_color(rgb(c.secondary))
                    .child(note.clone()),
            );
        }
        if let Some(details) = &frame.details {
            act!(
                div()
                    .id("welcome-details")
                    .debug_selector(|| "welcome-details".into())
                    .role(Role::DisclosureTriangle)
                    .aria_label("Details")
                    .aria_expanded(self.details_open)
                    .tab_index(0)
                    .when(frame.top, |d| d.mt(px(t.space.xl)))
                    .text_size(px(t.typography.body))
                    .text_color(rgb(c.secondary))
                    .child("Details")
                    .on_click(cx.listener(|v, _, _, cx| {
                        v.details_open = !v.details_open;
                        cx.notify();
                    })),
            );
            if self.details_open {
                act!(
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
        if two {
            // The action stands on the panel's bottom line.
            action = action.child(div().flex_1());
        }
        let mut footer = div()
            .flex()
            .flex_none()
            .w_full()
            .items_center()
            .gap(px(m.gap))
            .pt(px(if two {
                20.
            } else if frame.top {
                t.space.xl
            } else {
                m.inset
            }));
        for button in frame.buttons.iter().filter(|b| b.leading) {
            footer = footer.child(self.button(button, window, cx));
        }
        footer = if frame.centred && !two {
            footer.justify_center()
        } else {
            footer.child(div().flex_1())
        };
        for button in frame.buttons.iter().filter(|b| !b.leading) {
            footer = footer.child(self.button(button, window, cx));
        }
        act!(footer);
        let body = if two {
            self.columns(content, action, window).into_any_element()
        } else {
            content.into_any_element()
        };
        // The accessibility tree hangs off an identified, labelled root, as in
        // the editor; without it VoiceOver sees only the traffic lights.
        let mut root = div()
            .id("welcome")
            .role(Role::Group)
            .accessibility_id("shell.welcome")
            .aria_label(spoken)
            .relative()
            .map(|d| {
                if self.stage {
                    // The stage draws the window's ground; the card is its
                    // content, so a step's content cross-fades on one ground.
                    d.size_full()
                } else {
                    d.size_full().bg(rgb(c.window))
                }
            })
            .when(!two, |d| {
                d.flex().flex_col().items_center().p(px(m.sheet_inset))
            })
            .when(!two, |d| {
                if frame.top {
                    // Clear of the title bar, with the sheet's own air below.
                    d.justify_start()
                        .pt(px(if self.stage {
                            m.sheet_inset + m.inset
                        } else {
                            m.toolbar
                        }))
                        .pb(px(m.sheet_inset + m.inset))
                } else {
                    d.justify_center()
                }
            })
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
                    // On the stage the window is a real titled window: Back
                    // stands in its title bar, after the window buttons,
                    // centred on them.
                    .top(px(if self.stage {
                        TITLEBAR_MIDDLE - 9.
                    } else {
                        m.toolbar
                    }))
                    .left(px(if self.stage {
                        TITLEBAR_LEADING
                    } else {
                        m.sheet_inset
                    }))
                    .h(px(18.))
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
        root.child(body)
    }
}
/// Three quarters of a ring, one turn a second (`time` in turns). Still
/// under Reduce Motion. Also the kit's indeterminate progress row.
/// The Welcome window's title bar: the middle of its window buttons and
/// the first point after them (macOS 26 and 27, measured).
const TITLEBAR_MIDDLE: f32 = 16.;
const TITLEBAR_LEADING: f32 = 84.;
/// The stage's window in two columns: the left column's width (the right
/// is the action's panel), where both start under the title bar, and their
/// side insets.
pub const LEFT_COLUMN: f32 = 340.;
const COLUMN_TOP: f32 = 84.;
const COLUMN_INSET: f32 = 40.;
const PANEL_INSET: f32 = 40.;
/// A step's supporting line on the stage.
const LINE_SIZE: f32 = 14.;
/// The stage's one primary action per step: the extra-large glass button's
/// width and (drawn, headless) height.
const PRIMARY_WIDTH: f32 = 160.;
const PRIMARY_HEIGHT: f32 = 44.;
/// The primary action's distance from the window's bottom edge.
const PRIMARY_BOTTOM: f32 = 48.;
/// A step's title on the stage.
const STEP_TITLE: f32 = 28.;

pub fn spinner(time: f64, tokens: &Tokens) -> impl IntoElement + use<> {
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
/// The Powermove mark, `size` points high, filled with `color`.
fn mark_glyph(size: f32, color: u32) -> impl IntoElement + use<> {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let mark = &welcome::MARK;
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for p in mark.iter() {
                for k in 0..2 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            let scale = f64::from(size) / (hi[1] - lo[1]);
            let width = (hi[0] - lo[0]) * scale;
            let at = |x: f64, y: f64| {
                point(
                    bounds.origin.x
                        + px(((x - lo[0]) * scale) as f32)
                        + (bounds.size.width - px(width as f32)) / 2.,
                    bounds.origin.y + px(((y - lo[1]) * scale) as f32),
                )
            };
            let mut builder = PathBuilder::fill();
            builder.move_to(at(mark[0][0], mark[0][1]));
            for i in 0..mark.len() {
                let (a, b) = (mark[i], mark[(i + 1) % mark.len()]);
                builder.cubic_bezier_to(
                    at(b[0], b[1]),
                    at(a[0] + a[4], a[1] + a[5]),
                    at(b[0] + b[2], b[1] + b[3]),
                );
            }
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, rgb(color));
            }
        },
    )
    .w(px(size * 1.24))
    .h(px(size))
}

fn mark_canvas(time: f64, tokens: &Tokens) -> impl IntoElement + use<> {
    let accent = tokens.colors.accent;
    let ground = tokens.colors.window;
    let mark = canvas(
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
                ..Default::default()
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
    .size_full();
    div()
        .debug_selector(|| "welcome-mark".into())
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(mark)
}
/// Standard titled window, centred on the active display. Closing it quits.
pub fn open_welcome(
    tokens: Tokens,
    frame: Frame,
    on_action: OnAction,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<WelcomeView>> {
    open_welcome_window(tokens, frame, on_action, "Welcome to Powermove", true, cx)
}
/// The Welcome frame in its own window, for a step opened later (Settings ›
/// Storage › Bring In Extensions…): `quits` false closes just the window.
pub fn open_welcome_window(
    tokens: Tokens,
    frame: Frame,
    on_action: OnAction,
    title: &'static str,
    quits: bool,
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
                title: Some(title.into()),
                appears_transparent: true,
                ..Default::default()
            }))
            .is_resizable(false)
            .is_minimizable(false),
        |window, cx| {
            let view = cx.new(|cx| WelcomeView::new(tokens, frame, on_action, cx));
            window.on_window_should_close(cx, move |_, cx| {
                if quits {
                    cx.quit();
                }
                true
            });
            view.update(cx, |view, cx| {
                view.hold_focus(window, cx);
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

#[::core::prelude::v1::test]
#[ignore = "writes synthetic review captures"]
fn capture_utility_welcome() {
    use crate::welcome::{Choice, Names};
    crate::utility_capture_tests::capture("welcome", |_, cx| {
        cx.new(|cx| {
            WelcomeView::new(
                crate::tokens(),
                Frame {
                    title: "Welcome to Powermove".into(),
                    line: "Choose how to get started.".into(),
                    cards: true,
                    chosen: Some("new".into()),
                    choices: vec![
                        Choice {
                            id: "new".into(),
                            label: "Start Fresh".into(),
                            detail: None,
                            lines: vec!["Create a new library".into()],
                        },
                        Choice {
                            id: "import".into(),
                            label: "Bring Your Work".into(),
                            detail: None,
                            lines: vec!["Copy an existing library".into()],
                        },
                    ],
                    names: Some(Names {
                        title: "Projects".into(),
                        names: (1..=10).map(|i| format!("Project {i}")).collect(),
                        preview: 4,
                    }),
                    ..Frame::default()
                },
                Rc::new(|_, _| {}),
                cx,
            )
        })
    });
}
