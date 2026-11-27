//! The Welcome window: a standard titled window, title hidden, close button only.
//! Closing it quits the app; quitting at any point is safe and the next launch resumes.
use crate::welcome::{self, Frame, Progress, Severity, SignIn, SignInStage};
use crate::{
    kit::{Button, Kit, Recipes},
    motion::Motion,
    native_control::{ControlSpec, Kind, NativeControl},
};
use gpui::{prelude::*, *};
use pm_design::{
    Elevation, Tokens,
    motion::{Animated, Preset},
};
use std::{rc::Rc, time::Instant};

pub type OnAction = Rc<dyn Fn(&str, &mut App)>;
pub type ExtensionSlot = Rc<dyn Fn(&str, &Tokens, &mut Window, &mut App) -> Vec<AnyElement>>;

pub struct WelcomeView {
    motion: Motion<u64>,
    pub tokens: Tokens,
    pub frame: Frame,
    on_action: OnAction,
    extension_slot: Option<ExtensionSlot>,
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
    /// The two-column window outside the stage (a later launch's extension
    /// offer, recent work, a failure): it paints its own ground.
    pub ground: bool,
    /// The sign-in sheet (Settings, Home, the Store): one column in a
    /// small sheet in the app's appearance, the same sign-in as the stage's.
    pub sheet: bool,
    /// The sheet's Powermove tile, drawn once per appearance.
    sheet_tile: Option<(bool, std::sync::Arc<gpui::Image>)>,
    /// The sheet's content height as last laid out, and the window height
    /// last asked of AppKit for it (0 before the first).
    sheet_content: std::rc::Rc<std::cell::Cell<f32>>,
    sheet_height: f32,
    /// The stage card's AppKit buttons, by id and label.
    natives: std::collections::HashMap<(String, String), Entity<NativeControl>>,
    /// While a wave carries the card: the snapshot its buttons paint.
    snapshot: Option<u64>,
    snapshots: u64,
    /// The account step's email and code fields (`Frame::sign_in`).
    email_field: Option<Entity<NativeControl>>,
    code_field: Option<Entity<NativeControl>>,
    handle_field: Option<Entity<NativeControl>>,
    /// The sign-in's stage as last drawn: a new one gives its field the keyboard.
    sign_in_stage: Option<SignInStage>,
    /// Until when (the view's clock) the step's field asks for the keyboard
    /// again on each frame it does not have it: a control leaving with the
    /// step before (Use Email Instead) hands the keys back to the view after
    /// the field took them.
    field_focus_until: f64,
    /// The last wrong code's shake, and when it started.
    shake: (u64, f64),
    /// A long list of choices (the speech models) scrolls in its card.
    list_scroll: ScrollHandle,
}
impl WelcomeView {
    pub fn set_extension_slot(&mut self, render: ExtensionSlot, cx: &mut Context<Self>) {
        self.extension_slot = Some(render);
        cx.notify();
    }

    fn motion(&mut self) -> &mut Motion<u64> {
        &mut self.motion
    }
    pub fn new(tokens: Tokens, frame: Frame, on_action: OnAction, cx: &mut Context<Self>) -> Self {
        Self {
            motion: Motion::default(),
            tokens,
            frame,
            on_action,
            extension_slot: None,
            focus: cx.focus_handle(),
            opened: Instant::now(),
            started: 0.,
            details_open: false,
            names_open: false,
            body_scroll: ScrollHandle::new(),
            body_extent: None,
            clock: None,
            stage: false,
            ground: false,
            sheet: false,
            sheet_tile: None,
            sheet_content: Default::default(),
            sheet_height: 0.,
            natives: Default::default(),
            snapshot: None,
            snapshots: 0,
            email_field: None,
            code_field: None,
            handle_field: None,
            sign_in_stage: None,
            field_focus_until: 0.,
            shake: (0, 0.),
            list_scroll: ScrollHandle::new(),
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
    fn native_control(
        &mut self,
        button: &welcome::Button,
        size: Size,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<NativeControl> {
        let primary = size == Size::Primary;
        let key = (button.id.clone(), button.label.clone());
        let busy = primary && self.frame.busy;
        if let Some(control) = self.natives.get(&key) {
            let disabled = !button.enabled;
            control.update(cx, |c, cx| {
                c.set_busy(busy);
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
                    // On the stage the step's one action is filled with the
                    // accent, the column's width (Arc's Create Account).
                    tint: (primary && (self.stage || self.sheet)).then_some(tokens.colors.accent),
                    symbol: None,
                    glass: primary && !self.stage && !self.sheet,
                    well: false,
                },
                &tokens,
                window,
                cx,
            );
            control.set_build_snapshot(self.snapshot.map(crate::stage::snapshot_build));
            control.set_busy(busy);
            match size {
                Size::Primary | Size::Wide if self.sheet => control.wide(wt().sheet_inner()),
                Size::Primary if self.stage => control.prominent(wt().primary_width),
                Size::Pill => control.small(),
                _ => {}
            }
            if button.id == "google"
                && let Some(mark) = google_mark()
            {
                control.set_leading_image(&mark);
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
            // Back a step within the sign-in (the code to the email).
            "escape" if self.frame.sign_in.is_some() => self.act("escape", cx),
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
        if (self.stage || self.sheet) && crate::text::available(window) {
            return self
                .native_control(
                    button,
                    if primary {
                        Size::Primary
                    } else {
                        Size::Regular
                    },
                    window,
                    cx,
                )
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
        // Headless (no AppKit): the stage's accent button, drawn.
        element = if self.stage && primary {
            element
                .w(px(wt().primary_width))
                .h(px(wt().primary_height))
                .rounded_full()
                .text_size(px(ts().base))
                .font_weight(FontWeight::MEDIUM)
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
    /// The step's choices on the stage: one grouped list, its rows parted by
    /// hairlines inset from the leading edge. A row has its mark (one size,
    /// in a tile), its name and one quiet line, and at its end a capsule
    /// button (Connect, Download) or what the row says of itself. A list of
    /// plain options marks the chosen one with the accent's check, nothing
    /// else; a list with buttons (the agents) has no check.
    fn choice_list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = self.tokens.clone();
        let c = &t.colors;
        let choices = self.frame.choices.clone();
        let actions = choices
            .iter()
            .any(|c| c.pill.is_some() || c.status.is_some());
        let long = choices.len() > LIST_ROWS_SHOWN;
        let mut group = div()
            .id("welcome-choices")
            .debug_selector(|| "welcome-choices".into())
            .role(Role::RadioGroup)
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(wt().list_radius))
            .rounded_smoothing(t.radius.smoothing)
            .overflow_hidden()
            .bg(rgba(alpha(c.text, ink().well)));
        // More rows than the column holds over the pinned actions (the
        // speech models): the card keeps its height and its rows scroll in
        // it, an edge fading where there is more to see. Nothing else moves.
        if long {
            let offset = -f32::from(self.list_scroll.offset().y);
            let room = f32::from(self.list_scroll.max_offset().y);
            // The first layout says how far it can scroll: one frame more
            // draws the lower fade.
            if room == 0. && self.clock.is_none() && !self.list_scroll.bounds().is_empty() {
                window.request_animation_frame();
            }
            let fade = |on: bool| px(if on { wt().list_fade } else { 0. }).into();
            group = group
                // Each row's name, tag and meta on their two lines: the
                // card takes some of the column's margin (still well inside
                // the half).
                .w(px(wt().column + 2. * wt().list_widen))
                .ml(px(-wt().list_widen))
                .max_h(px(wt().list_max_height))
                .overflow_y_scroll()
                .track_scroll(&self.list_scroll)
                .overflow_fade(Edges::<AbsoluteLength> {
                    top: fade(offset > 1.),
                    bottom: fade(room == 0. || offset < room - 1.),
                    left: px(0.).into(),
                    right: px(0.).into(),
                })
                .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()));
        }
        for (index, choice) in choices.iter().enumerate() {
            let selected = self.frame.chosen.as_ref() == Some(&choice.id);
            let id = choice.id.clone();
            // In a card that scrolls, neither a row nor its hairline gives
            // up height to the card.
            if index > 0 {
                group = group.child(
                    div()
                        .flex_none()
                        .ml(px(wt().list_inset))
                        .h(px(0.5))
                        .bg(rgba(alpha(c.text, ink().edge))),
                );
            }
            let mark = choice.mark.clone().map(|name| {
                let glyph = if name == "powermove" {
                    mark_glyph(17., c.accent).into_any_element()
                } else {
                    // A mark in its own colour (Claude's orange); others in
                    // the text's ink.
                    let ink = if name == "provider.claude" {
                        crate::scale().colors.brand_claude
                    } else {
                        c.text
                    };
                    crate::symbol::symbol(&name, 20., crate::symbol::Weight::Regular, ink, window)
                        .into_any_element()
                };
                div()
                    .flex_none()
                    .size(px(wt().icon_tile))
                    .rounded(px(t.radius.nested(wt().list_radius, 6.)))
                    .bg(rgba(alpha(c.text, ink().well)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(glyph)
            });
            let pill = choice
                .pill
                .clone()
                .map(|label| self.pill(&format!("pill:{}", choice.id), &label, window, cx));
            // A tag with a detail beside it (a speech model) sits after the
            // name, smaller, so the row keeps two lines: the name, then the
            // detail on one line. A tag alone (Powermove's agent) stands in
            // for the detail, under the name.
            let inline_tag = choice.badge.is_some() && choice.detail.is_some();
            // A long list's rows are all two lines at one height.
            let one_line = inline_tag || long;
            let tag = |badge: String, size: f32| {
                div()
                    .flex_none()
                    .px(px(5.))
                    .rounded(px(4.))
                    .bg(rgba(alpha(c.accent, ink().tint)))
                    .text_size(px(size))
                    .line_height(px(14.))
                    .text_color(rgb(c.accent))
                    .font_weight(FontWeight::MEDIUM)
                    .child(badge)
            };
            let subtitle = div()
                .min_w(px(0.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .text_size(px(ts().xs))
                .line_height(px(14.))
                .text_color(rgb(c.secondary))
                .children(
                    choice
                        .badge
                        .clone()
                        .filter(|_| !inline_tag)
                        .map(|badge| tag(badge, 11.)),
                )
                .children(choice.detail.clone().map(|detail| {
                    // One line beside an inline tag's name; else it wraps in
                    // the text column.
                    div()
                        .min_w(px(0.))
                        .when(one_line, |d| d.flex_1().truncate())
                        .when(!one_line, |d| d.max_w_full())
                        .child(detail)
                }));
            let row = div()
                .id(SharedString::from(format!("choice-{}", choice.id)))
                .debug_selector(|| format!("choice-{id}", id = id.clone()))
                .role(Role::RadioButton)
                .aria_label(choice.label.clone())
                .aria_selected(selected)
                .tab_index(0)
                .flex_none()
                .min_h(px(wt().list_row))
                .when(long, |d| d.h(px(wt().list_row)))
                .px(px(wt().list_inset))
                .py(px(8.))
                .flex()
                .items_center()
                .gap(px(10.))
                .hover_style(|s| s.bg(rgba(alpha(c.text, ink().wash))))
                .children(mark)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            // One line: a long name gives way to the row's
                            // status or pill, never wraps under it; an
                            // inline tag follows it.
                            div()
                                .min_w(px(0.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .truncate()
                                        .text_size(px(ts().base))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(c.text))
                                        .child(choice.label.clone()),
                                )
                                .children(
                                    choice
                                        .badge
                                        .clone()
                                        .filter(|_| inline_tag)
                                        .map(|badge| tag(badge, 10.)),
                                ),
                        )
                        .child(subtitle),
                )
                .children(choice.status.clone().map(|status| {
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_size(px(ts().xs))
                        .text_color(rgb(c.secondary))
                        .child(status)
                }))
                .children(pill)
                .when(selected && !actions, |d| {
                    d.child(crate::symbol::symbol(
                        "checkmark",
                        12.,
                        crate::symbol::Weight::Semibold,
                        c.accent,
                        window,
                    ))
                })
                .on_click(cx.listener(move |v, _, _, cx| v.act(&format!("choose:{id}"), cx)));
            group = group.child(row);
        }
        group.into_any_element()
    }
    /// A row's capsule button: `id` is its action.
    fn pill(
        &mut self,
        id: &str,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let button = welcome::Button::new(id, label);
        if crate::text::available(window) {
            return self
                .native_control(&button, Size::Pill, window, cx)
                .into_any_element();
        }
        // Headless (no AppKit): the same capsule, drawn.
        let t = &self.tokens;
        let id = id.to_owned();
        div()
            .id(SharedString::from(format!("welcome-{id}")))
            .role(Role::Button)
            .aria_label(label.to_owned())
            .flex_none()
            .h(px(22.))
            .px(px(11.))
            .flex()
            .items_center()
            .rounded_full()
            .bg(rgba(alpha(t.colors.text, ink().edge)))
            .text_size(px(ts().sm))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(t.colors.text))
            .child(label.to_owned())
            .on_click(cx.listener(move |v, _, _, cx| v.act(&id, cx)))
            .into_any_element()
    }
    /// The Welcome window on the stage, built as Arc's Create Account
    /// window is: on the left a solid dark half with one centred column
    /// (the mark, how far along the steps are, the title, a line, the step's
    /// controls, its one primary action and a quiet link under it); on the
    /// right a field of our colour with the product, cropped by the window's
    /// edge (`preview`).
    fn stage_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        self.motion.frame(window);
        // The sign-in's stages spring in: the title and line rise into place
        // as a stage changes, the fields and lines under them follow.
        let sign_in = self.frame.sign_in.clone();
        let title_p = match &sign_in {
            Some(s) => {
                let mut p = 1.;
                for stage in [
                    SignInStage::Email,
                    SignInStage::Code,
                    SignInStage::Done,
                    SignInStage::Handle,
                    SignInStage::Browser,
                ] {
                    let shown = self.shown(
                        TITLE_KEY + stage as u64,
                        s.stage == stage,
                        Preset::Show,
                        window,
                    );
                    if s.stage == stage {
                        p = shown;
                    }
                }
                p
            }
            None => 1.,
        };
        let sign_in = sign_in.map(|s| self.sign_in_block(&s, window, cx));
        let kit = Kit::new(&self.tokens, &self.motion, Self::motion, window, cx);
        let t = self.tokens.clone();
        let c = &t.colors;
        let frame = self.frame.clone();
        let spin = frame.spinner && !t.accessibility.reduce_motion;
        let sliding = !t.accessibility.reduce_motion
            && frame
                .steps
                .iter()
                .any(|s| s.progress == Some(Progress::Indeterminate));
        if (spin || sliding) && self.clock.is_none() {
            window.request_animation_frame();
        }
        let spoken = frame.title.replace('\n', " ");
        // The mark, title, line and content, from the mark's height down to
        // where the actions start.
        let mut column = div()
            .w(px(wt().column))
            .h(px((wt().actions_top - wt().actions_room) - wt().logo_top))
            .flex()
            .flex_col()
            .items_center()
            .text_center()
            .child(mark_glyph(26., c.accent));
        column = column
            .child(
                div()
                    .id("welcome-title")
                    .debug_selector(|| "welcome-title".into())
                    .role(Role::Heading)
                    .aria_label(spoken.clone())
                    .mt(px(24.))
                    .text_size(px(ts().x4l))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(relative(1.15))
                    .letter_spacing(px(-0.4))
                    .text_color(rgb(c.text))
                    .relative()
                    .top(px((1. - title_p) * 8.))
                    .opacity(title_p.clamp(0., 1.))
                    .child(frame.title.clone()),
            )
            .child(
                div()
                    .id("welcome-line")
                    .role(Role::Label)
                    .aria_label(frame.line.replace('\n', " "))
                    .mt(px(8.))
                    .text_size(px(ts().base))
                    .line_height(px(18.))
                    .text_color(rgb(c.secondary))
                    .relative()
                    .top(px((1. - title_p) * 12.))
                    .opacity(title_p.clamp(0., 1.))
                    .child(frame.line.clone()),
            );
        // The step's controls.
        let mut controls = div().w_full().flex().flex_col().gap(px(12.)).text_left();
        if let Some(sign_in) = sign_in {
            controls = controls.child(sign_in);
        }
        if let Some(status) = &frame.status {
            let mut row = div()
                .id("welcome-status")
                .debug_selector(|| "welcome-status".into())
                .role(Role::Label)
                .aria_label(status.clone())
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .text_size(px(ts().base))
                .text_color(rgb(c.secondary));
            if frame.spinner {
                row = row.child(spinner(if spin { self.time() } else { 0. }, &t));
            }
            controls = controls.child(row.child(status.clone()));
        }
        if !frame.stats.is_empty() {
            // One quiet line ("38 brought in · 1 couldn’t load"): the column
            // keeps its room for the list and the actions.
            let line = frame
                .stats
                .iter()
                .map(|stat| {
                    let mut label = stat.label.clone();
                    if let Some(first) = label.get(..1)
                        && !label.starts_with("Powermove")
                    {
                        label.replace_range(..1, &first.to_lowercase());
                    }
                    format!("{} {label}", stat.value)
                })
                .collect::<Vec<_>>()
                .join(" · ");
            controls = controls.child(
                div()
                    .id("welcome-stats")
                    .role(Role::Label)
                    .aria_label(line.clone())
                    .text_center()
                    .text_size(px(ts().base))
                    .text_color(rgb(c.secondary))
                    .child(line),
            );
        }
        if !frame.steps.is_empty() {
            controls = controls.child(self.steps(&kit, sliding.then(|| self.time())));
        }
        if !frame.choices.is_empty() {
            controls = controls.child(self.choice_list(window, cx));
        }
        if !frame.rows.is_empty() || frame.names.is_some() {
            let mut body = div()
                .id("welcome-body")
                .debug_selector(|| "welcome-body".into())
                .flex()
                .flex_col()
                .gap(px(12.))
                .max_h(px(220.))
                .overflow_y_scroll()
                .track_scroll(&self.body_scroll);
            if !frame.rows.is_empty() {
                body = body.child(self.rows(window));
            }
            if let Some(names) = &frame.names {
                body = body.child(self.names(&kit, names, cx));
            }
            controls = controls.child(body);
        }
        // The sign-in draws its own error, under its field.
        if let Some(error) = frame.error.as_ref().filter(|_| frame.sign_in.is_none()) {
            controls = controls.child(
                div()
                    .text_size(px(ts().base))
                    .text_color(rgb(c.danger))
                    .text_center()
                    .child(error.clone()),
            );
        }
        if let Some(note) = &frame.note {
            controls = controls.child(
                div()
                    .text_size(px(ts().sm))
                    .text_color(rgb(c.secondary))
                    .text_center()
                    .child(note.clone()),
            );
        }
        if let Some(details) = &frame.details {
            controls = controls.child(
                div()
                    .id("welcome-details")
                    .debug_selector(|| "welcome-details".into())
                    .role(Role::DisclosureTriangle)
                    .aria_label("Details")
                    .aria_expanded(self.details_open)
                    .tab_index(0)
                    .text_size(px(ts().base))
                    .text_color(rgb(c.secondary))
                    .text_center()
                    .child("Details")
                    .on_click(cx.listener(|v, _, _, cx| {
                        v.details_open = !v.details_open;
                        cx.notify();
                    })),
            );
            if self.details_open {
                controls = controls.child(
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
        let has_controls = !frame.choices.is_empty()
            || frame.sign_in.is_some()
            || frame.status.is_some()
            || !frame.steps.is_empty()
            || !frame.stats.is_empty()
            || !frame.rows.is_empty()
            || frame.names.is_some()
            || frame.error.is_some()
            || frame.note.is_some()
            || frame.details.is_some();
        if has_controls {
            // Between the title and the actions: a list taller than the
            // room scrolls here; it never moves the actions.
            column = column.child(
                div()
                    .id("welcome-content")
                    .debug_selector(|| "welcome-content".into())
                    // The column's width, and room either side for a long
                    // list's wider card (its scroll clips what overhangs).
                    .w(px(wt().column + 2. * wt().list_widen))
                    .flex_none()
                    .px(px(wt().list_widen))
                    .mt(px(22.))
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .child(controls),
            );
        }
        // One primary action, the column's width; the others, and Back, as
        // quiet links under it.
        let primary = frame
            .default
            .as_ref()
            .and_then(|id| frame.buttons.iter().find(|b| &b.id == id))
            .or_else(|| frame.buttons.iter().find(|b| !b.leading));
        // The actions sit at one place on every frame (the owner: the next
        // button never moves): the primary, then the links under it.
        let mut actions = div()
            .id("welcome-actions")
            .debug_selector(|| "welcome-actions".into())
            .absolute()
            .left_0()
            .w(px(wt().half))
            .top(px(wt().actions_top))
            .flex()
            .flex_col()
            .items_center();
        actions = actions.child(
            div()
                .id("welcome-primary")
                .debug_selector(|| "welcome-primary".into())
                .h(px(wt().primary_height))
                .flex()
                .justify_center()
                .children(
                    primary
                        .cloned()
                        .map(|button| self.button(&button, window, cx)),
                ),
        );
        let mut links: Vec<(String, String)> = frame
            .buttons
            .iter()
            .filter(|b| Some(&b.id) != primary.map(|p| &p.id))
            .map(|b| (b.id.clone(), b.label.clone()))
            .collect();
        if frame.back.is_some() {
            links.push(("back".into(), "Back".into()));
        }
        if !links.is_empty() {
            actions = actions.child(
                div()
                    .id("welcome-links")
                    .debug_selector(|| "welcome-links".into())
                    .mt(px(12.))
                    .flex()
                    .justify_center()
                    .gap(px(18.))
                    .children(links.into_iter().map(|(id, label)| {
                        let act = id.clone();
                        div()
                            .id(SharedString::from(format!("welcome-{id}")))
                            .debug_selector(move || format!("welcome-{id}"))
                            .role(Role::Link)
                            .aria_label(label.clone())
                            .tab_index(0)
                            .text_size(px(ts().base))
                            .text_color(rgb(c.secondary))
                            .hover_style(|s| s.text_color(rgb(c.text)))
                            .child(label)
                            .on_click(cx.listener(move |v, _, _, cx| v.act(&act, cx)))
                    })),
            );
        }
        let left = div()
            .absolute()
            .left_0()
            .top_0()
            .bottom_0()
            .w(px(wt().half))
            .flex()
            .flex_col()
            .items_center()
            // The mark at one height on every step; the step under it.
            .pt(px(wt().logo_top))
            .child(column);
        let [top_slots, bottom_slots] = ["welcome.top", "welcome.bottom"].map(|id| {
            self.extension_slot.as_ref()
                .map_or_else(Vec::new, |render| render(id, &t, window, cx))
        });
        let right = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .left(px(wt().half))
            .overflow_hidden()
            .children(frame.preview.as_deref().and_then(|id| preview(id, window)))
            .when(!top_slots.is_empty(), |d| {
                d.child(div().absolute().top(px(t.metrics.titlebar)).left_0().right_0()
                    .flex().flex_col().children(top_slots))
            })
            .when(!bottom_slots.is_empty(), |d| {
                d.child(div().absolute().bottom_0().left_0().right_0()
                    .flex().flex_col().children(bottom_slots))
            });
        div()
            .id("welcome")
            .role(Role::Group)
            .accessibility_id("shell.welcome")
            .aria_label(spoken)
            .relative()
            .size_full()
            // The system font: SF Pro, its Display cut at the title's size
            // (automatic optical sizing).
            .font_family(pm_design::TEXT_FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .when(self.ground, |d| d.child(stage_ground()))
            .child(left)
            .child(right)
            .child(actions)
    }
    /// The sign-in sheet, one column 32 pt in from its edges: the Powermove
    /// tile, the title and one line; Continue with Google first, "or",
    /// the email (or the code, the handle, the check) and Continue, each the
    /// column's width; Cancel, quiet, at the foot. The sign-in block is the
    /// stage's (fields, cells, shake, resend), in the app's appearance.
    fn sheet_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        self.motion.frame(window);
        let t = self.tokens.clone();
        let c = &t.colors;
        let frame = self.frame.clone();
        let dark = t.scheme == pm_design::Scheme::Dark;
        if self.sheet_tile.as_ref().is_none_or(|(d, _)| *d != dark) {
            self.sheet_tile =
                crate::settings_window::tile_png("powermove", dark, wt().sheet_tile as f64, 2.)
                    .map(|png| {
                        (
                            dark,
                            std::sync::Arc::new(gpui::Image::from_bytes(
                                gpui::ImageFormat::Png,
                                png,
                            )),
                        )
                    });
        }
        let stage = frame.sign_in.as_ref().map(|s| s.stage);
        let title_p = match stage {
            Some(now) => {
                let mut p = 1.;
                for stage in [
                    SignInStage::Email,
                    SignInStage::Code,
                    SignInStage::Done,
                    SignInStage::Handle,
                    SignInStage::Browser,
                ] {
                    let shown =
                        self.shown(TITLE_KEY + stage as u64, now == stage, Preset::Show, window);
                    if now == stage {
                        p = shown;
                    }
                }
                p
            }
            None => 1.,
        };
        let sign_in = frame
            .sign_in
            .clone()
            .map(|s| self.sign_in_block(&s, window, cx));
        let spoken = frame.title.replace('\n', " ");
        let (tile_w, _) = crate::settings_window::tile_image_size(wt().sheet_tile as f64);
        let find = |id: &str| frame.buttons.iter().find(|b| b.id == id).cloned();
        let google = find("google");
        let primary = frame
            .default
            .as_ref()
            .and_then(|id| frame.buttons.iter().find(|b| &b.id == id))
            .cloned();
        // The browser step's way back stands where Continue does.
        let secondary = primary.is_none().then(|| find("back")).flatten();
        let links: Vec<(String, String)> = frame
            .buttons
            .iter()
            .filter(|b| {
                Some(&b.id) != primary.as_ref().map(|p| &p.id)
                    && b.id != "google"
                    && Some(&b.id) != secondary.as_ref().map(|p| &p.id)
            })
            .map(|b| (b.id.clone(), b.label.clone()))
            .collect();
        let mut column = div()
            .w(px(wt().sheet_inner()))
            .flex()
            .flex_col()
            .items_center()
            .text_center()
            .children(self.sheet_tile.as_ref().map(|(_, image)| {
                div()
                    .id("sign-in-tile")
                    .role(Role::Image)
                    .aria_label("Powermove")
                    .size(px(tile_w as f32))
                    .child(gpui::img(image.clone()).size(px(tile_w as f32)))
            }))
            .child(
                div()
                    .id("welcome-title")
                    .debug_selector(|| "welcome-title".into())
                    .role(Role::Heading)
                    .aria_label(spoken.clone())
                    .mt(px(10.))
                    .text_size(px(ts().x2l))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(relative(1.2))
                    .letter_spacing(px(-0.2))
                    .text_color(rgb(c.text))
                    .relative()
                    .top(px((1. - title_p) * 6.))
                    .opacity(title_p.clamp(0., 1.))
                    .child(frame.title.clone()),
            )
            .child(
                div()
                    .id("welcome-line")
                    .role(Role::Label)
                    .aria_label(frame.line.replace('\n', " "))
                    .mt(px(6.))
                    .text_size(px(ts().base))
                    .line_height(px(18.))
                    .text_color(rgb(c.secondary))
                    .relative()
                    .top(px((1. - title_p) * 10.))
                    .opacity(title_p.clamp(0., 1.))
                    .child(frame.line.clone()),
            );
        // Google first, then "or" between hairlines, on the email step.
        if let Some(button) = google {
            let hairline = || div().flex_1().h(px(1.)).bg(rgba(alpha(c.text, ink().edge)));
            column = column
                .child(
                    div()
                        .id("welcome-google")
                        .mt(px(24.))
                        .h(px(wt().sheet_control))
                        .flex()
                        .justify_center()
                        .child(self.native_control(&button, Size::Wide, window, cx)),
                )
                .child(
                    div()
                        .id("sign-in-or")
                        .role(Role::Label)
                        .aria_label("or")
                        .w_full()
                        .mt(px(16.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .text_size(px(ts().sm))
                        .text_color(rgb(c.secondary))
                        .child(hairline())
                        .child("or")
                        .child(hairline()),
                );
        }
        column = column.children(sign_in.map(|block| {
            div()
                .w_full()
                .mt(px(if stage == Some(SignInStage::Email) {
                    16.
                } else {
                    24.
                }))
                .child(block)
        }));
        if let Some(button) = primary.or(secondary.clone()) {
            let size = if secondary.is_some() {
                Size::Wide
            } else {
                Size::Primary
            };
            column = column.child(
                div()
                    .id("welcome-primary")
                    .debug_selector(|| "welcome-primary".into())
                    .mt(px(12.))
                    .h(px(wt().sheet_control))
                    .flex()
                    .justify_center()
                    .child(self.native_control(&button, size, window, cx)),
            );
        }
        let foot = div()
            .id("welcome-links")
            .debug_selector(|| "welcome-links".into())
            .mt(px(wt().sheet_foot_gap))
            .h(px(18.))
            .flex()
            .justify_center()
            .gap(px(18.))
            .children(links.into_iter().map(|(id, label)| {
                let act = id.clone();
                div()
                    .id(SharedString::from(format!("welcome-{id}")))
                    .debug_selector(move || format!("welcome-{id}"))
                    .role(Role::Link)
                    .aria_label(label.clone())
                    .tab_index(0)
                    .text_size(px(ts().base))
                    .text_color(rgb(c.secondary))
                    .hover_style(|s| s.text_color(rgb(c.text)))
                    .child(label)
                    .on_click(cx.listener(move |v, _, _, cx| v.act(&act, cx)))
            }));
        self.fit_sheet(window, cx);
        let content = self.sheet_content.clone();
        // The sheet is as tall as its step: the column and Cancel are
        // measured as laid out, and the window follows on the next frame.
        let measure = gpui::canvas(
            move |bounds, window, _| {
                let height = f32::from(bounds.size.height);
                if (height - content.get()).abs() > 0.5 {
                    content.set(height);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div()
            .id("welcome")
            .role(Role::Dialog)
            .accessibility_id("shell.sign-in")
            .aria_label(spoken)
            .relative()
            .size_full()
            // Settings' card colour: the sheet stands as one of its cards.
            .bg(rgb(c.panel))
            .font_family(pm_design::TEXT_FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .items_center()
            .pt(px(wt().sheet_padding))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(column)
                    .child(foot)
                    .child(measure),
            )
    }
    /// Asks AppKit for the height the step's content needs, about the
    /// sheet's centre: at once the first time, animated after (AppKit's own
    /// sheets grow and shrink this way).
    fn fit_sheet(&mut self, window: &mut Window, cx: &mut App) {
        let content = self.sheet_content.get();
        if content <= 0. {
            return;
        }
        let height = (wt().sheet_padding + content + wt().sheet_foot).round();
        if (height - self.sheet_height).abs() < 0.5 {
            return;
        }
        let first = self.sheet_height == 0.;
        self.sheet_height = height;
        let Some(ns) = crate::settings_window::gpui_view(window).and_then(|v| v.window()) else {
            return;
        };
        let animate = !first && !self.tokens.accessibility.reduce_motion;
        // Not while GPUI draws: after this update's effects.
        window.defer(cx, move |_, _| {
            let frame = ns.frame();
            let content = ns.contentRectForFrameRect(frame);
            let grow = height as f64 - content.size.height;
            if grow.abs() < 0.5 {
                return;
            }
            // macOS centres a sheet on its window: it grows about its centre.
            let mut next = frame;
            next.size.height += grow;
            next.origin.y -= (grow / 2.).round();
            ns.setFrame_display_animate(next, true, animate);
        });
    }
    /// How far `key` has sprung in (0–1, or past it on an overshoot): from
    /// 0 the first time it is shown, back towards 0 while it is not, so a
    /// stage that comes back mid-way carries on from where it was.
    fn shown(&mut self, key: u64, shown: bool, preset: Preset, window: &mut Window) -> f32 {
        if self.tokens.accessibility.reduce_motion {
            return if shown { 1. } else { 0. };
        }
        let spring = self.tokens.spring(preset);
        let mut values = self.motion.values.borrow_mut();
        let value = values.entry(key).or_insert_with(|| Animated::unit(0.));
        let target = if shown { 1. } else { 0. };
        if value.target[0] != target {
            value.to(target, spring);
        }
        // Set after this frame's step (`Motion::frame`): the next frames
        // carry it on.
        let now = value.get();
        if (now - target).abs() > 0.001 && self.clock.is_none() {
            window.request_animation_frame();
        }
        now
    }
    /// The sign-in's field for `which` (`email` or `code`), made once: a
    /// real AppKit text field, its typing and keys sent to the flow.
    fn text_field(
        &mut self,
        which: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<NativeControl> {
        let held = match which {
            "code" => &self.code_field,
            "handle" => &self.handle_field,
            _ => &self.email_field,
        };
        if let Some(field) = held {
            return field.clone();
        }
        let tokens = self.tokens.clone();
        let code = which == "code";
        let identifier = format!("welcome-{which}");
        let field = cx.new(|cx| {
            let mut field = NativeControl::new(
                ControlSpec {
                    kind: Kind::Text,
                    label: match which {
                        "code" => "",
                        "handle" => "your-name",
                        _ => "name@example.com",
                    },
                    identifier: &identifier,
                    value: &serde_json::Value::String(String::new()),
                    options: vec![],
                    disabled: false,
                    default: false,
                    destructive: false,
                    inline: false,
                    quiet: false,
                    pull: false,
                    tint: None,
                    // The sheet's email and the handle start empty: AutoFill
                    // never fills or selects them.
                    symbol: Some(if code || !self.sheet { which } else { "plain" }),
                    glass: false,
                    well: false,
                },
                &tokens,
                window,
                cx,
            );
            if code {
                // The cells are drawn here; the field under them keeps the
                // keyboard, a pasted code and VoiceOver.
                field.alpha = 0.;
                field.set_height(wt().cell_height);
                field.describe("6-digit code");
            } else {
                field.borderless();
                field.describe(if which == "handle" { "Handle" } else { "Email" });
            }
            field
        });
        cx.subscribe_in(
            &field,
            window,
            move |v: &mut Self, _, value: &serde_json::Value, window, cx| match value {
                serde_json::Value::String(text) => v.act(&format!("field:{which}:{text}"), cx),
                serde_json::Value::Object(key) => match key.get("key").and_then(|k| k.as_str()) {
                    Some("Enter") => {
                        if let Some(id) = v.frame.default_action().map(str::to_owned) {
                            v.act(&id, cx);
                        }
                    }
                    Some("Escape") => v.act("escape", cx),
                    // On to the actions: the keyboard is the window's again.
                    Some("Tab") => window.focus(&v.focus, cx),
                    _ => {}
                },
                _ => {}
            },
        )
        .detach();
        match which {
            "code" => self.code_field = Some(field.clone()),
            "handle" => self.handle_field = Some(field.clone()),
            _ => self.email_field = Some(field.clone()),
        }
        field
    }
    /// The account's sign-in under the title: the email field; then the
    /// email as a line with Change over the code's six cells and Resend; then
    /// a check. Its error, if any, under it.
    fn sign_in_block(
        &mut self,
        s: &SignIn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The sheet's fields run its column's width at its buttons' height.
        let (well_w, well_h) = if self.sheet {
            (wt().sheet_inner(), wt().sheet_control)
        } else {
            (wt().primary_width, wt().field_height)
        };
        let t = self.tokens.clone();
        let c = &t.colors;
        let now = self.now();
        let arrived = self.sign_in_stage.replace(s.stage) != Some(s.stage);
        if arrived {
            self.field_focus_until = now + FIELD_FOCUS_SECONDS;
        }
        let claim = now < self.field_focus_until;
        // A test's fixed clock never runs out: no frames for it.
        if claim && self.clock.is_none() {
            window.request_animation_frame();
        }
        if s.shake != self.shake.0 {
            self.shake = (s.shake, now);
        }
        let shake_t = (now - self.shake.1) as f32;
        let shaking = s.shake > 0 && shake_t < SHAKE_SECONDS && !t.accessibility.reduce_motion;
        if shaking && self.clock.is_none() {
            window.request_animation_frame();
        }
        // One shake: a quick sideways swing that dies away.
        let dx = if shaking {
            8. * (-shake_t * 7.).exp() * (shake_t * std::f32::consts::TAU * 6.).sin()
        } else {
            0.
        };
        let email_p = self.shown(
            SIGN_IN_KEY,
            s.stage == SignInStage::Email,
            Preset::Show,
            window,
        );
        let code_p = self.shown(
            SIGN_IN_KEY + 1,
            s.stage == SignInStage::Code,
            Preset::Show,
            window,
        );
        let done_p = self.shown(
            SIGN_IN_KEY + 2,
            s.stage == SignInStage::Done,
            Preset::Pop,
            window,
        );
        let handle_p = self.shown(
            SIGN_IN_KEY + 3,
            s.stage == SignInStage::Handle,
            Preset::Show,
            window,
        );
        let browser_p = self.shown(
            SIGN_IN_KEY + 4,
            s.stage == SignInStage::Browser,
            Preset::Show,
            window,
        );
        let link = |id: &'static str, label: &str, cx: &mut Context<Self>| {
            div()
                .id(SharedString::from(format!("welcome-{id}")))
                .debug_selector(move || format!("welcome-{id}"))
                .role(Role::Link)
                .aria_label(label.to_owned())
                .tab_index(0)
                .text_color(rgb(c.text))
                .hover_style(|s| s.opacity(0.75))
                .child(label.to_owned())
                .on_click(cx.listener(move |v, _, _, cx| v.act(id, cx)))
        };
        let mut block = div()
            .id("welcome-sign-in")
            .debug_selector(|| "welcome-sign-in".into())
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .text_size(px(ts().base));
        match s.stage {
            SignInStage::Email => {
                let field = self.text_field("email", window, cx);
                let focused = field.update(cx, |f, cx| {
                    f.host_text(&s.email, s.revision);
                    f.alpha = email_p.clamp(0., 1.);
                    if arrived || (claim && !f.focused()) {
                        f.focus_on_paint();
                    }
                    cx.notify();
                    f.focused()
                });
                // The field's well, as the code's cells: a quiet fill and a
                // hairline, the edge lifting while it has the keyboard.
                block = block.child(
                    div()
                        .id("welcome-email-well")
                        .debug_selector(|| "welcome-email-well".into())
                        .w(px(well_w))
                        .h(px(well_h))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .rounded(px(wt().field_radius))
                        .bg(rgba(alpha(c.text, ink().well)))
                        .border_1()
                        .border_color(if focused {
                            rgb(c.focus)
                        } else {
                            rgba(alpha(c.text, ink().edge))
                        })
                        .relative()
                        .top(px((1. - email_p) * 10.))
                        .opacity(email_p.clamp(0., 1.))
                        .child(div().flex_1().child(field)),
                );
            }
            SignInStage::Code => {
                // The email, risen from its field into a line.
                block = block.child(
                    div()
                        .id("welcome-sent-to")
                        .debug_selector(|| "welcome-sent-to".into())
                        .flex()
                        .gap(px(6.))
                        .relative()
                        .top(px((1. - code_p) * 26.))
                        .opacity(code_p.clamp(0., 1.))
                        .child(
                            div()
                                .id("welcome-sent-email")
                                .role(Role::Label)
                                .aria_label(s.email.clone())
                                .text_color(rgb(c.secondary))
                                .child(s.email.clone()),
                        )
                        .child(div().text_color(rgb(c.faint)).child("·"))
                        .child(link("change", "Change", cx)),
                );
                let field = self.text_field("code", window, cx);
                let focused = field.update(cx, |f, cx| {
                    f.host_text(&s.code, s.revision);
                    if arrived || (claim && !f.focused()) {
                        f.focus_on_paint();
                    }
                    cx.notify();
                    f.focused()
                });
                let digits: Vec<char> = s.code.chars().collect();
                let active = digits.len().min(CELLS - 1);
                let cells = (0..CELLS).map(|i| {
                    let digit = digits.get(i).copied();
                    let current = focused && i == active;
                    div()
                        .w(px(wt().cell_width))
                        .h(px(wt().cell_height))
                        // The middle of the code: "123 456".
                        .when(i == CELLS / 2, |d| d.ml(px(8.)))
                        .rounded(px(wt().field_radius))
                        .bg(rgba(alpha(c.text, ink().well)))
                        .border_1()
                        .border_color(if current {
                            rgb(c.focus)
                        } else {
                            rgba(alpha(c.text, ink().edge))
                        })
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(ts().x3l))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c.text))
                        .children(digit.map(|d| d.to_string()))
                        .when(current && digit.is_none(), |d| {
                            d.child(div().w(px(1.5)).h(px(22.)).bg(rgb(c.text)))
                        })
                });
                block = block.child(
                    div()
                        .id("welcome-code")
                        .debug_selector(|| "welcome-code".into())
                        .relative()
                        .left(px(dx))
                        .top(px((1. - code_p) * 14.))
                        .opacity(code_p.clamp(0., 1.))
                        .child(div().flex().gap(px(8.)).children(cells))
                        .child(div().absolute().inset_0().child(field)),
                );
            }
            SignInStage::Handle => {
                let field = self.text_field("handle", window, cx);
                let focused = field.update(cx, |f, cx| {
                    f.host_text(&s.handle, s.revision);
                    f.alpha = handle_p.clamp(0., 1.);
                    if arrived || (claim && !f.focused()) {
                        f.focus_on_paint();
                    }
                    cx.notify();
                    f.focused()
                });
                // The email's well, an @ before the name.
                block = block.child(
                    div()
                        .id("welcome-handle-well")
                        .debug_selector(|| "welcome-handle-well".into())
                        .w(px(well_w))
                        .h(px(well_h))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .rounded(px(wt().field_radius))
                        .bg(rgba(alpha(c.text, ink().well)))
                        .border_1()
                        .border_color(if focused {
                            rgb(c.focus)
                        } else {
                            rgba(alpha(c.text, ink().edge))
                        })
                        .relative()
                        .top(px((1. - handle_p) * 10.))
                        .opacity(handle_p.clamp(0., 1.))
                        .child(div().text_color(rgb(c.secondary)).child("@"))
                        .child(div().flex_1().child(field)),
                );
            }
            SignInStage::Browser => {
                let spin = !t.accessibility.reduce_motion;
                if spin && self.clock.is_none() {
                    window.request_animation_frame();
                }
                block = block.child(
                    div()
                        .id("welcome-browser")
                        .debug_selector(|| "welcome-browser".into())
                        .role(Role::Label)
                        .aria_label("Waiting for your browser")
                        .h(px(wt().field_height))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_color(rgb(c.secondary))
                        .opacity(browser_p.clamp(0., 1.))
                        .child(spinner(if spin { self.time() } else { 0. }, &t))
                        .child("Finish in your browser"),
                );
            }
            SignInStage::Done => {
                let size = 44. * (0.7 + 0.3 * done_p);
                block = block.child(
                    div()
                        .id("welcome-signed-in")
                        .debug_selector(|| "welcome-signed-in".into())
                        .role(Role::Image)
                        .aria_label("Signed in")
                        .h(px(52.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .size(px(size))
                                .rounded_full()
                                .bg(rgb(c.success))
                                .opacity(done_p.clamp(0., 1.))
                                .child(check(size, done_p)),
                        ),
                );
            }
        }
        if let Some(error) = &self.frame.error {
            // The line wraps in the column; Try again under it.
            block = block.child(
                div()
                    .id("welcome-sign-in-error")
                    .debug_selector(|| "welcome-sign-in-error".into())
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .id("welcome-sign-in-error-line")
                            .role(Role::Alert)
                            .aria_label(error.clone())
                            .w_full()
                            .text_center()
                            .line_height(px(18.))
                            .text_color(rgb(c.danger))
                            .child(error.clone()),
                    )
                    .when(s.retry, |d| d.child(link("retry", "Try again", cx))),
            );
        }
        if s.stage == SignInStage::Code
            && let Some(wait) = s.resend_in
        {
            block = block.child(
                div()
                    .id("welcome-resend")
                    .debug_selector(|| "welcome-resend".into())
                    .flex()
                    .gap(px(5.))
                    .text_color(rgb(c.secondary))
                    .opacity(code_p.clamp(0., 1.))
                    .map(|d| {
                        if wait == 0 {
                            d.child("Didn’t get it?")
                                .child(link("resend", "Resend code", cx))
                        } else {
                            let label = format!("Resend code in {wait} s");
                            d.role(Role::Label)
                                .aria_label(label.clone())
                                .text_color(rgb(c.faint))
                                .child(label)
                        }
                    }),
            );
        }
        block.into_any_element()
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
        if self.stage {
            return self.stage_window(window, cx);
        }
        if self.sheet {
            return self.sheet_window(window, cx);
        }
        self.motion.frame(window);
        let kit = Kit::new(&self.tokens, &self.motion, Self::motion, window, cx);
        let t = self.tokens.clone();
        let (c, m) = (&t.colors, &t.metrics);
        let frame = self.frame.clone();
        // On the stage the film is the animation.
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
        let spoken = frame.title.replace('\n', " ");
        // A step's screens (`top`) keep their title at one height and their
        // buttons at the bottom; the others centre as one block.
        let air = px(t.space.xxl);
        let mut content = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(m.gap))
            .w_full()
            .max_w(px(m.sheet_width_wide))
            .max_h(relative(1.))
            .min_h(px(0.))
            .when(frame.top, |d| d.h_full().gap(px(0.)))
            .when(frame.centred, |d| d.items_center().text_center());
        let mut header = div()
            .flex()
            .flex_col()
            .gap(px(if frame.top { m.gap + 2. } else { m.gap }))
            .when(frame.centred, |d| d.items_center());
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
                        |d| d.text_size(px(t.typography.display)),
                    )
                    .text_color(rgb(c.text))
                    .child(frame.title.clone()),
            )
            .child(
                div()
                    .id("welcome-line")
                    .role(Role::Label)
                    .aria_label(frame.line.clone())
                    .text_size(px(t.typography.reading))
                    .line_height(relative(1.45))
                    .text_color(rgb(c.secondary))
                    .child(frame.line.clone()),
            );
        content = content.child(header);
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
            content = content.child(spaced(div()).child(row.child(status.clone())));
        }
        if !frame.stats.is_empty() {
            content = content.child(spaced(div().flex_none()).child(self.stats(&kit)));
        }
        if !frame.steps.is_empty() {
            content = content.child(
                spaced(div().flex_none()).child(self.steps(&kit, sliding.then(|| self.time()))),
            );
        }
        if !frame.choices.is_empty() {
            content = content.child(self.choices(&kit, cx));
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
            let window_colour = rgba(alpha(c.window, 1.));
            let clear = rgba(alpha(c.window, 0.));
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
            content = content.child(wrapper);
        } else if frame.top {
            content = content.child(div().flex_1());
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
                    .when(frame.top, |d| d.mt(px(t.space.xl)))
                    .text_size(px(t.typography.body))
                    .line_height(relative(1.45))
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
        let mut footer = div()
            .flex()
            .flex_none()
            .w_full()
            .gap(px(m.gap))
            .pt(px(if frame.top { t.space.xl } else { m.inset }));
        for button in frame.buttons.iter().filter(|b| b.leading) {
            footer = footer.child(self.button(button, window, cx));
        }
        footer = if frame.centred {
            footer.justify_center()
        } else {
            footer.child(div().flex_1())
        };
        for button in frame.buttons.iter().filter(|b| !b.leading) {
            footer = footer.child(self.button(button, window, cx));
        }
        content = content.child(footer);
        // The accessibility tree hangs off an identified, labelled root, as in
        // the editor; without it VoiceOver sees only the traffic lights.
        let mut root = div()
            .id("welcome")
            .role(Role::Group)
            .accessibility_id("shell.welcome")
            .aria_label(spoken)
            .relative()
            .size_full()
            .bg(rgb(c.window))
            .flex()
            .flex_col()
            .items_center()
            .p(px(m.sheet_inset))
            .map(|d| {
                if frame.top {
                    // Clear of the title bar, with the sheet's own air below.
                    d.justify_start()
                        .pt(px(m.toolbar))
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
/// A native button's size on the stage.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Size {
    /// The step's one primary action: the column's width, the accent.
    Primary,
    /// Any other button.
    Regular,
    /// A row's small button (Connect, Download): a capsule.
    Pill,
    /// The sign-in sheet's other full-width button (Continue with Google).
    Wide,
}
// Every place, size and colour the stage and the sheet draw at is a token
// (`pm_design::Welcome`, `TextSize`, `Ink`, `Colors`).
fn wt() -> pm_design::Welcome {
    crate::scale().welcome
}
fn ts() -> pm_design::TextSize {
    crate::scale().text_size
}
fn ink() -> pm_design::Ink {
    crate::scale().ink
}
/// `rgb` with an `Ink` step's alpha, for `rgba`.
fn alpha(rgb: u32, a: f32) -> u32 {
    pm_design::with_alpha(rgb, a)
}
/// A list longer than this scrolls in its card (`list_max_height`).
const LIST_ROWS_SHOWN: usize = 4;
/// The sign-in sheet's size (its height follows its step).
pub fn sheet_size() -> (f32, f32) {
    (wt().sheet_width, wt().sheet_height)
}
/// How long a step's field keeps asking for the keyboard after it arrives.
const FIELD_FOCUS_SECONDS: f64 = 0.5;
/// The sign-in's six code cells, as wide together as the primary action.
const CELLS: usize = 6;
const SHAKE_SECONDS: f32 = 0.45;
/// `Motion::values` keys: the sign-in's title by stage, and its parts.
const TITLE_KEY: u64 = 0x5160_0000;
const SIGN_IN_KEY: u64 = 0x5160_0010;

/// Three quarters of a ring, one turn a second (`time` in turns). Still
/// under Reduce Motion. Also the kit's indeterminate progress row.
/// The signed-in check, drawn in as `p` reaches 1.
fn check(size: f32, p: f32) -> impl IntoElement + use<> {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let at = |x: f32, y: f32| bounds.origin + point(px(x * size), px(y * size));
            let points = [at(0.30, 0.52), at(0.44, 0.66), at(0.71, 0.37)];
            let mut builder = PathBuilder::stroke(px(size / 15.));
            builder.move_to(points[0]);
            // The stroke's two legs, the second drawn as the first ends.
            let drawn = p.clamp(0., 1.) * 2.;
            builder.line_to(points[0] + (points[1] - points[0]) * drawn.min(1.));
            if drawn > 1. {
                builder.line_to(points[1] + (points[2] - points[1]) * (drawn - 1.));
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, rgb(crate::scale().colors.white));
            }
        },
    )
    .size(px(size))
}
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
/// The stage's Welcome window's ground (`stage` paints it, so a step's
/// change moves only the step): the dark half and the field of our colour.
pub fn stage_ground() -> Div {
    div()
        .absolute()
        .inset_0()
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(wt().half))
                .bg(rgb(crate::scale().colors.welcome_ground)),
        )
        .child(field())
}
/// The right half's field: our orange with depth. A warm gradient from a lit
/// top to an ember base, the base deepened further, a soft light from the
/// upper left, and a fine grain so it reads as a surface, not a fill.
fn field() -> Div {
    let light = div()
        .absolute()
        .left(px(-140.))
        .top(px(-200.))
        .size(px(420.))
        .rounded_full()
        .shadow(vec![BoxShadow {
            color: hsla(36. / 360., 1., 0.82, 0.42).into(),
            offset: point(px(0.), px(0.)),
            blur_radius: px(220.),
            spread_radius: px(40.),
            inset: false,
        }]);
    let mut grain = div().absolute().inset_0().opacity(0.07).overflow_hidden();
    if let Some((image, (w, h))) = picture("grain") {
        let (w, h) = (w as f32 / 2., h as f32 / 2.);
        let (columns, rows) = ((wt().half / w).ceil() as usize, (900. / h).ceil() as usize);
        for row in 0..rows {
            for column in 0..columns {
                grain = grain.child(
                    img(ImageSource::Render(image.clone()))
                        .absolute()
                        .left(px(column as f32 * w))
                        .top(px(row as f32 * h))
                        .w(px(w))
                        .h(px(h)),
                );
            }
        }
    }
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .left(px(wt().half))
        .overflow_hidden()
        .bg(linear_gradient(
            160.,
            linear_color_stop(rgb(crate::scale().colors.welcome_field_light), 0.),
            linear_color_stop(rgb(crate::scale().colors.welcome_field_deep), 1.),
        ))
        .child(div().absolute().inset_0().bg(linear_gradient(
            180.,
            linear_color_stop(hsla(14. / 360., 0.9, 0.12, 0.), 0.45),
            linear_color_stop(hsla(14. / 360., 0.9, 0.12, 0.38), 1.),
        )))
        .child(light)
        .child(grain)
}

/// A picture from `assets/onboarding`, decoded once (BGRA, for GPUI).
fn picture(name: &str) -> Option<(std::sync::Arc<RenderImage>, (u32, u32))> {
    use std::{cell::RefCell, collections::HashMap};
    thread_local! {
        static IMAGES: RefCell<HashMap<String, Option<(std::sync::Arc<RenderImage>, (u32, u32))>>> = RefCell::default();
    }
    let bytes: &'static [u8] = match name {
        "grain" => include_bytes!("../assets/onboarding/grain.png"),
        "welcome" => include_bytes!("../assets/onboarding/welcome.png"),
        "timeline-layers" => include_bytes!("../assets/onboarding/timeline-layers.png"),
        "timeline-tracks" => include_bytes!("../assets/onboarding/timeline-tracks.png"),
        "agent" => include_bytes!("../assets/onboarding/agent.png"),
        "speech" => include_bytes!("../assets/onboarding/speech.png"),
        "workspace" => include_bytes!("../assets/onboarding/workspace.png"),
        "home" => include_bytes!("../assets/onboarding/home.png"),
        "effects" => include_bytes!("../assets/onboarding/effects.png"),
        _ => return None,
    };
    let decode = || {
        let mut rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .ok()?
            .into_rgba8();
        let size = rgba.dimensions();
        // GPUI's sprites are BGRA.
        for p in rgba.pixels_mut() {
            p.0.swap(0, 2);
        }
        Some((
            std::sync::Arc::new(RenderImage::new([image::Frame::new(rgba)])),
            size,
        ))
    };
    IMAGES.with(|c| {
        c.borrow_mut()
            .entry(name.to_owned())
            .or_insert_with(decode)
            .clone()
    })
}
/// The product beside a step: a real Powermove window (a lane capture of the
/// app on a synthetic project, framed on the step's subject) floating on the
/// field at a little under its size, with its own corners, a hairline rim and
/// a soft shadow, running off the field's right and bottom edges.
fn preview(id: &str, window: &Window) -> Option<AnyElement> {
    let _ = window;
    let (image, (w, h)) = picture(id)?;
    // The captures are two pixels a point; shown at `wt().window_scale`.
    let (w, h) = (
        w as f32 / 2. * wt().window_scale,
        h as f32 / 2. * wt().window_scale,
    );
    let radius = 16. * wt().window_scale;
    Some(
        div()
            .absolute()
            .left(px(wt().window_left))
            .top(px(wt().window_top))
            .w(px(w))
            .h(px(h))
            .rounded(px(radius))
            .shadow(vec![
                BoxShadow {
                    color: hsla(14. / 360., 0.8, 0.08, 0.55).into(),
                    offset: point(px(0.), px(28.)),
                    blur_radius: px(64.),
                    spread_radius: px(-8.),
                    inset: false,
                },
                BoxShadow {
                    color: hsla(0., 0., 0., 0.25).into(),
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(6.),
                    spread_radius: px(0.),
                    inset: false,
                },
            ])
            .child(
                div()
                    .size_full()
                    .rounded(px(radius))
                    .overflow_hidden()
                    .border(px(0.5))
                    .border_color(hsla(0., 0., 1., 0.22))
                    .child(img(ImageSource::Render(image)).size_full()),
            )
            .into_any_element(),
    )
}

/// The Powermove mark, `size` points high, filled with `color`.
/// Google's multicolour G, as its sign-in branding has it on a sign-in
/// button (16 pt, drawn once).
fn google_mark() -> Option<objc2::rc::Retained<objc2_app_kit::NSImage>> {
    thread_local! {
        static MARK: Option<objc2::rc::Retained<objc2_app_kit::NSImage>> = crate::settings_window::svg_image(
            include_str!("../icons/brands/google-g.svg"),
            objc2_foundation::NSSize::new(16., 16.),
        );
    }
    MARK.with(Clone::clone)
}
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
    // The same two-column window as the first-run steps: dark whatever the
    // system's appearance (its left half is the stage's), with the
    // person's accessibility settings.
    let dark = |accessibility| Tokens::new(pm_design::Scheme::Dark, accessibility);
    let tokens = dark(tokens.accessibility);
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
        move |window, cx| {
            let view = cx.new(|cx| {
                let mut view = WelcomeView::new(tokens, frame, on_action, cx);
                view.stage = true;
                view.ground = true;
                view
            });
            window.on_window_should_close(cx, move |_, cx| {
                if quits {
                    cx.quit();
                }
                true
            });
            view.update(cx, |view, cx| {
                view.hold_focus(window, cx);
                cx.observe_window_appearance(window, move |v, _, cx| {
                    v.tokens = dark(crate::macos::tokens().accessibility);
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
                            ..Choice::default()
                        },
                        Choice {
                            id: "import".into(),
                            label: "Bring Your Work".into(),
                            detail: None,
                            lines: vec!["Copy an existing library".into()],
                            ..Choice::default()
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
