//! The first-run stage: one clear, borderless window over the whole display.
//! The person's own desktop and windows dim under a black veil that fades in
//! (Dia's measured opacity and curve), a quiet credit shows on the dim, the
//! Powermove mark rises from below the bottom edge into the centre as a lit
//! object, and the distortion wave draws the Welcome window out of the mark's
//! light, once, on the window's own rectangle. Nothing paints a ground: the
//! stage is the veil, the mark and its light (`light`'s one pass,
//! premultiplied over clear), the credit and the window. The first-run steps
//! change inside the window with a quick spring cross-fade. Once the Welcome
//! stands in its own real window the veil and the light fade away and the
//! stage leaves (`Curtain`): nothing stays above the person's other apps.
//! The last step hands off into the app: the window eases out.
//! Timing is the cue sheet's (`cue`), Dia's opening measured.
pub mod cue;
pub mod intro;
pub mod light;
pub mod native_window;

use crate::kit::Recipes;
use crate::native_control::{BuildSnapshot, ControlSpec, Kind, NativeControl};
use crate::wave::{DistortionWave, ReducedMotion, WaveRect, WaveState};
use crate::welcome_view::WelcomeView;
use cue::smooth;
pub use cue::{CueSheet, Phase};
use gpui::{prelude::*, *};
use pm_design::{
    Elevation, Tokens,
    motion::{Animated, Spring, UNIT_PRECISION},
};
use std::{rc::Rc, time::Instant};

/// A sound cue (`cue::Sound::id`) reached.
pub type OnSound = Rc<dyn Fn(&str, &mut App)>;

/// Seconds the window takes to fade in after a Skip.
const SKIP_FADE: f64 = 0.35;
/// The original's Reduce Motion cross-fade, seconds.
const REDUCED_WAVE: f64 = 0.3;
/// The hand-off: the window eases out while the veil fades (`cue::dim_out`).
const WINDOW_OUT: f64 = 0.32;
/// A step's content arrives from a few points below and leaves a few up.
const SLIDE_IN: f32 = 10.;
const SLIDE_OUT: f32 = 6.;
/// quiet-md's quick fade (critically damped) and its vanish.
const ARRIVE: Spring = Spring::new(520., 46.);
const LEAVE: Spring = Spring::new(900., 60.);

/// The stage's lights after the wave: they go down once the Welcome stands
/// in its own real window (`CueSheet::lights_left`), and whatever is left
/// fades with the hand-off (`CueSheet::veil_left`). Dark once, then the
/// stage window goes; it never comes back.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Curtain {
    /// The real window showed (the intro's time).
    down: Option<f64>,
    /// The hand-off started.
    out: Option<f64>,
    gone: bool,
}
impl Curtain {
    /// The Welcome's real window is up at `t`: the lights go down.
    pub fn native_shown(&mut self, t: f64) {
        self.down.get_or_insert(t);
    }
    /// The hand-off starts at `t`.
    pub fn hand_off(&mut self, t: f64) {
        self.out.get_or_insert(t);
    }
    /// Seconds into the hand-off at `t`, once it has started.
    pub fn out_since(&self, t: f64) -> Option<f64> {
        self.out.map(|at| t - at)
    }
    pub fn handing_off(&self) -> bool {
        self.out.is_some()
    }
    /// The share of the veil and the mark's light left at `t` (1 to 0).
    pub fn left(&self, sheet: &CueSheet, motion: bool, t: f64) -> f64 {
        if self.gone {
            return 0.;
        }
        let down = self.down.map_or(1., |at| sheet.lights_left(t - at, motion));
        let out = self.out.map_or(1., |at| sheet.veil_left(t - at));
        (down * out).clamp(0., 1.)
    }
    /// The lights are on their way down: the stage keeps drawing.
    pub fn fading(&self) -> bool {
        !self.gone && (self.down.is_some() || self.out.is_some())
    }
    /// True once, when the lights are out at `t` (or at once with `force`,
    /// the safety net for a stage that stopped drawing): the stage goes.
    pub fn take_dark(&mut self, sheet: &CueSheet, motion: bool, t: f64, force: bool) -> bool {
        if !self.fading() || !(force || self.left(sheet, motion, t) <= 0.) {
            return false;
        }
        self.gone = true;
        true
    }
    pub fn gone(&self) -> bool {
        self.gone
    }
}
/// A step's content leaving while the next arrives.
struct Leaving {
    card: Entity<WelcomeView>,
    presence: Animated,
}

pub struct Stage {
    pub sheet: CueSheet,
    pub tokens: Tokens,
    light: light::LightState,
    opened: Instant,
    /// Tests and recordings pin the clock (seconds).
    pub clock: Option<f64>,
    /// Captures wait for the light's renderer instead of skipping frames.
    pub wait: bool,
    /// Added to the clock: a Skip moves the intro to the window.
    offset: f64,
    skipped_at: Option<f64>,
    card: Option<Entity<WelcomeView>>,
    /// The arriving step's presence (0–1).
    presence: Animated,
    leaving: Option<Leaving>,
    last_frame: Option<f64>,
    wave: Entity<WaveState>,
    /// The wave has its pictures (taken during the hold).
    prepared: bool,
    last_t: f64,
    on_sound: Option<OnSound>,
    focus: FocusHandle,
    skip: Option<Entity<NativeControl>>,
    curtain: Curtain,
    /// `StageEvent::WindowLive` was sent.
    live_sent: bool,
    /// A real window (`native_window`) holds the Welcome now: the stage
    /// draws only the veil and the light, once that window has drawn its
    /// first frame.
    native: Option<AnyWindowHandle>,
    native_drawn: bool,
    /// The step made for the real window, drawn only there until it shows
    /// (its AppKit controls belong to that window).
    pending: Option<Entity<WelcomeView>>,
}

/// What the stage tells its owner.
pub enum StageEvent {
    /// The wave has drawn the window and it is live: the owner puts the
    /// Welcome in a real window on the same rectangle (`native_window`).
    WindowLive,
    /// The lights are out (`Curtain`): the owner removes the stage window.
    Dark,
}
impl EventEmitter<StageEvent> for Stage {}

impl Stage {
    /// `intro` false: the window at once (a launch with nothing to welcome).
    pub fn new(tokens: Tokens, sheet: CueSheet, intro: bool, cx: &mut Context<Self>) -> Self {
        let wave = cx.new(|_| {
            WaveState::new(
                cue::wave_style(),
                WaveRect {
                    x: 0.,
                    y: 0.,
                    w: 1.,
                    h: 1.,
                },
            )
            .expect("the original wave's defaults are valid")
        });
        let mut sheet = sheet;
        if tokens.accessibility.reduce_motion {
            // Reduce Motion: the wave is the original's plain cross-fade.
            sheet.wave_visible = REDUCED_WAVE / sheet.wave_seconds;
        }
        let offset = if intro { 0. } else { sheet.length() };
        Self {
            sheet,
            tokens,
            light: light::LightState::default(),
            opened: Instant::now(),
            clock: None,
            wait: false,
            offset,
            skipped_at: None,
            card: None,
            presence: Animated::unit(1.),
            leaving: None,
            last_frame: None,
            wave,
            prepared: false,
            last_t: -1.,
            on_sound: None,
            focus: cx.focus_handle(),
            skip: None,
            curtain: Curtain::default(),
            live_sent: false,
            native: None,
            native_drawn: false,
            pending: None,
        }
    }
    pub fn on_sound(&mut self, sound: OnSound) {
        self.on_sound = Some(sound);
    }
    fn raw(&self) -> f64 {
        self.clock
            .unwrap_or_else(|| self.opened.elapsed().as_secs_f64())
    }
    /// The intro's time: seconds since the stage opened, moved on by Skip.
    pub fn now(&self) -> f64 {
        self.raw() + self.offset
    }
    pub fn card(&self) -> Option<&Entity<WelcomeView>> {
        self.card.as_ref()
    }
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }
    /// The Welcome window's content; once the window is live, a new step
    /// cross-fades in over the one before (interruptible: a step arriving
    /// mid-change takes over from wherever the last one was).
    pub fn set_card(&mut self, card: Entity<WelcomeView>, cx: &mut Context<Self>) {
        // A step arriving before the real window shows takes the place of
        // the one made for it, there.
        if self.pending.is_some() {
            self.pending = Some(card);
            cx.notify();
            return;
        }
        let live = self.now() >= self.sheet.length();
        if let Some(previous) = self.card.replace(card.clone())
            && live
        {
            let mut presence = Animated::new([self.presence.get()], LEAVE, UNIT_PRECISION);
            presence.to(0., LEAVE);
            previous.update(cx, |c, cx| c.set_snapshot(true, cx));
            card.update(cx, |c, cx| c.set_snapshot(true, cx));
            self.leaving = Some(Leaving {
                card: previous,
                presence,
            });
            self.presence = Animated::new([0.], ARRIVE, UNIT_PRECISION);
            self.presence.to(1., ARRIVE);
        }
        cx.notify();
    }
    /// The Welcome moves into a real window (`window`), with `card` (made
    /// for that window, so its AppKit controls live there) in place of the
    /// current one, at once.
    pub fn host_natively(
        &mut self,
        window: AnyWindowHandle,
        card: Entity<WelcomeView>,
        cx: &mut Context<Self>,
    ) {
        self.native = Some(window);
        self.native_drawn = false;
        self.pending = Some(card);
        cx.notify();
    }
    /// The real window is up: the stage stops drawing its own copy, and
    /// its veil and light fade away; then it goes.
    pub fn native_shown(&mut self, cx: &mut Context<Self>) {
        self.native_drawn = true;
        if let Some(card) = self.pending.take() {
            self.card = Some(card);
            self.leaving = None;
            self.presence = Animated::unit(1.);
        }
        self.curtain.native_shown(self.now());
        // Never left standing if the stage stops drawing (an occluded
        // window, a Space switch): dark by this time whatever happens.
        let wait = self.sheet.lights_seconds(self.motion()) + 0.45;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs_f64(wait))
                .await;
            let _ = this.update(cx, |stage, cx| stage.lights_out(true, cx));
        })
        .detach();
        cx.notify();
    }
    /// Dark (or `force`): tells the owner, once.
    fn lights_out(&mut self, force: bool, cx: &mut Context<Self>) {
        let (motion, t) = (self.motion(), self.now());
        if self.curtain.take_dark(&self.sheet, motion, t, force) {
            cx.emit(StageEvent::Dark);
        }
    }
    /// The real window holding the Welcome, if one does.
    pub fn native_window(&self) -> Option<AnyWindowHandle> {
        self.native
    }
    /// The Welcome window's rectangle on a stage of `area`.
    pub fn welcome_rect(&self, area: Size<Pixels>) -> Bounds<Pixels> {
        self.window_rect(area)
    }
    /// The window's content for the real window: its ground and the step
    /// (or a step change), filling `window`. Returns whether it has drawn
    /// (the window is shown from then on).
    pub fn window_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> (AnyElement, bool) {
        let t = self.now();
        if self.step_springs(t, cx) {
            window.request_animation_frame();
        }
        let mut ground = div()
            .id("stage-window")
            .debug_selector(|| "stage-window".into())
            .relative()
            .size_full()
            .overflow_hidden()
            .child(crate::welcome_view::stage_ground());
        ground = match &self.pending {
            Some(card) => ground.child(div().absolute().inset_0().child(card.clone())),
            None => self.contents(ground),
        };
        (ground.into_any_element(), true)
    }
    /// Esc or Skip: straight to the window, with a short fade.
    pub fn skip(&mut self, cx: &mut Context<Self>) {
        let t = self.now();
        let window = self.sheet.length();
        if t >= window {
            return;
        }
        self.offset += window - t;
        self.skipped_at = Some(window);
        self.wave.update(cx, |w, _| w.clear());
        cx.notify();
    }
    /// The last step: the window eases out while what is left of the veil
    /// fades from the app's first window behind the stage; dark, the stage
    /// tells its owner (`StageEvent::Dark`).
    pub fn hand_off(&mut self, cx: &mut Context<Self>) {
        let start = self.now().max(self.sheet.length());
        self.offset += start - self.now();
        self.curtain.hand_off(start);
        cx.notify();
    }
    fn motion(&self) -> bool {
        !self.tokens.accessibility.reduce_motion
    }
    fn sounds(&mut self, t: f64, cx: &mut Context<Self>) {
        if t > self.last_t {
            let ids: Vec<&'static str> = self
                .sheet
                .sounds_between(self.last_t, t)
                .map(|s| s.id)
                .collect();
            // A sound skipped over (Skip, a stalled frame) stays silent.
            if t - self.last_t < 0.5
                && let Some(play) = &self.on_sound
            {
                for id in ids {
                    let (play, id) = (play.clone(), id.to_owned());
                    cx.defer(move |cx| play(&id, cx));
                }
            }
        }
        self.last_t = t;
    }
    /// The mark and its light at `t` (clear elsewhere), filling the stage.
    fn room(&mut self, t: f64, area: Size<Pixels>, scale: f32) -> AnyElement {
        let size = (
            (f32::from(area.width) * scale).round(),
            (f32::from(area.height) * scale).round(),
        );
        // Once the wave has the window, the light holds as it was at the hold
        // (the wave draws over that very picture): nothing around the window
        // changes as it arrives. The mark itself is under the window.
        let at = if t >= self.sheet.length() {
            self.sheet.hold
        } else {
            t
        };
        let mut light = intro::light_at(at, &self.sheet, self.motion(), size, scale);
        if t >= self.sheet.length() {
            light.mark = 0.;
        }
        let texture = self.light.picture(light, self.wait);
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                if let Some(texture) = texture {
                    window.paint_texture(bounds, texture);
                }
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }
    /// The intro at `t`: the mark and its light, the credit, the wordmark,
    /// on nothing (the veil under them is the stage's).
    fn intro(&mut self, t: f64, area: Size<Pixels>, scale: f32) -> AnyElement {
        let motion = self.motion();
        div()
            .id("stage-intro")
            .relative()
            .size_full()
            .overflow_hidden()
            .child(self.room(t, area, scale))
            .child(intro::credit(t, &self.sheet, area, scale))
            .child(intro::words(t, &self.sheet, area, &self.tokens, motion))
            .into_any_element()
    }
    /// The Welcome window's rectangle on the stage: centred.
    fn window_rect(&self, area: Size<Pixels>) -> Bounds<Pixels> {
        let m = &self.tokens.metrics;
        let size = size(px(m.welcome_width), px(m.welcome_height));
        Bounds::new(
            point(
                (area.width - size.width) / 2.,
                (area.height - size.height) / 2.,
            ),
            size,
        )
    }
    /// The real Welcome window's corner (a titled window without a toolbar,
    /// measured on macOS 26 and 27): the wave reveals the window it becomes.
    fn window_radius(&self) -> f32 {
        16.
    }
    /// The window: its ground and the current step's content, with a
    /// leaving step's content fading over it.
    fn window(&self, rect: Bounds<Pixels>, shadow: f32) -> Stateful<Div> {
        let t = &self.tokens;
        let mut ground = div()
            .id("stage-window")
            .debug_selector(|| "stage-window".into())
            .relative()
            .w(rect.size.width)
            .h(rect.size.height)
            .corner(t, self.window_radius())
            .border_1()
            .border_color(rgba((t.colors.text << 8) | 0x16))
            .overflow_hidden()
            .child(crate::welcome_view::stage_ground());
        if shadow > 0. {
            // The shadow arrives with the window, not with its picture.
            ground = faded_shadow(ground.elevated(t, Elevation::Raised), shadow);
        }
        self.contents(ground)
    }
    /// The step on the window's ground, with a leaving step fading over it.
    fn contents(&self, mut ground: Stateful<Div>) -> Stateful<Div> {
        let content = |card: &Entity<WelcomeView>, presence: f32, slide: f32| {
            div()
                .absolute()
                .inset_0()
                .top(px(slide))
                .opacity(presence)
                .child(card.clone())
        };
        if let Some(leaving) = &self.leaving {
            let p = leaving.presence.get().clamp(0., 1.);
            ground = ground.child(content(&leaving.card, p, -SLIDE_OUT * (1. - p)));
        }
        if let Some(card) = &self.card {
            let p = self.presence.get().clamp(0., 1.);
            ground = ground.child(content(card, p, SLIDE_IN * (1. - p)));
        }
        ground
    }
    /// The window standing on the dim at `t`.
    fn window_scene(
        &mut self,
        t: f64,
        area: Size<Pixels>,
        scale: f32,
        out: f64,
    ) -> AnyElement {
        let room = div()
            .relative()
            .size_full()
            .child(self.room(t, area, scale));
        if self.native.is_some() && self.native_drawn {
            // The real window holds the Welcome (and eases itself out).
            return room.into_any_element();
        }
        let rect = self.window_rect(area);
        // Out: the window eases away, settling a few points as it goes.
        let w = self
            .window(rect, 1.)
            .top(px(8. * out as f32))
            .opacity(1. - out as f32);
        room.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(w),
            )
            .into_any_element()
    }
    /// The wave on the window's rectangle: the intro under it becomes the
    /// window, rising out of the mark's light; the stage around it holds.
    fn wave_scene(
        &mut self,
        t: f64,
        area: Size<Pixels>,
        scale: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hold = self.sheet.hold;
        let rect = self.window_rect(area);
        if !self.prepared {
            self.prepared = true;
            let reduced = !self.motion();
            // The wave starts from the mark: its rectangle in the window.
            let mark = intro::rest();
            let h = f64::from(area.height);
            let (mw, mh) = (mark.height() * h * 138. / 112., mark.height() * h);
            let source = WaveRect {
                x: mark.x * f64::from(area.width) - mw / 2. - f64::from(rect.origin.x),
                y: mark.y * h - mh / 2. - f64::from(rect.origin.y),
                w: mw,
                h: mh,
            };
            self.wave.update(cx, |w, _| {
                w.clear();
                let _ = w.set_source(source);
                // The stage's own setting (the person's, or a capture's).
                let motion = if reduced {
                    ReducedMotion::Always
                } else {
                    ReducedMotion::System
                };
                w.set_reduced_motion(motion, window);
                w.prepare_now();
            });
            if let Some(card) = &self.card {
                card.update(cx, |c, cx| c.set_snapshot(true, cx));
            }
        }
        let run = if self.motion() {
            self.sheet.wave_seconds
        } else {
            REDUCED_WAVE
        };
        let p = (t - self.sheet.window) / run;
        if p > 0. {
            let _ = self.wave.update(cx, |w, _| w.seek(p.min(1. - 1e-4)));
        }
        let under = div()
            .relative()
            .w(rect.size.width)
            .h(rect.size.height)
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .left(-rect.origin.x)
                    .top(-rect.origin.y)
                    .w(area.width)
                    .h(area.height)
                    .child(self.intro(hold, area, scale)),
            );
        let radius = self.window_radius();
        let to = self.window(rect, 0.);
        let shadow = smooth((p - 0.2) / 0.3) as f32;
        // The intro around the window's rectangle only: inside it the wave
        // draws its own picture of it (its light would otherwise add twice).
        let mut stage = div().relative().size_full();
        let (x0, y0) = (rect.origin.x, rect.origin.y);
        let (x1, y1) = (x0 + rect.size.width, y0 + rect.size.height);
        for (l, t, w, h) in [
            (px(0.), px(0.), area.width, y0),
            (px(0.), y1, area.width, area.height - y1),
            (px(0.), y0, x0, rect.size.height),
            (x1, y0, area.width - x1, rect.size.height),
        ] {
            stage = stage.child(
                div()
                    .absolute()
                    .left(l)
                    .top(t)
                    .w(w)
                    .h(h)
                    .overflow_hidden()
                    .child(
                        div()
                            .absolute()
                            .left(-l)
                            .top(-t)
                            .w(area.width)
                            .h(area.height)
                            .child(self.intro(hold, area, scale)),
                    ),
            );
        }
        if shadow > 0. {
            // The window's shadow comes in under the wave's last light.
            stage = stage.child(
                div()
                    .absolute()
                    .left(rect.origin.x)
                    .top(rect.origin.y)
                    .w(rect.size.width)
                    .h(rect.size.height)
                    .rounded(px(radius))
                    .map(|d| faded_shadow(d.elevated(&self.tokens, Elevation::Raised), shadow)),
            );
        }
        stage
            .child(
                div()
                    .absolute()
                    .left(rect.origin.x)
                    .top(rect.origin.y)
                    .child(
                        DistortionWave::new(self.wave.clone(), under)
                            .revealed_content(to)
                            .w(rect.size.width)
                            .h(rect.size.height)
                            .rounded(px(radius)),
                    ),
            )
            .into_any_element()
    }
    fn set_snapshots(&self, on: bool, cx: &mut Context<Self>) {
        for card in self
            .card
            .iter()
            .chain(self.leaving.as_ref().map(|l| &l.card))
        {
            card.update(cx, |c, cx| c.set_snapshot(on, cx));
        }
    }
    /// The step change's springs, on the stage's clock.
    fn step_springs(&mut self, t: f64, cx: &mut Context<Self>) -> bool {
        let dt = self.last_frame.map_or(0., |last| (t - last).max(0.)) as f32;
        self.last_frame = Some(t);
        let mut moving = self.presence.step(dt);
        if let Some(leaving) = &mut self.leaving {
            moving |= leaving.presence.step(dt);
        }
        if !moving && self.leaving.take().is_some() {
            self.set_snapshots(false, cx);
        }
        moving
    }
    fn skip_button(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if crate::text::available(window) {
            if self.skip.is_none() {
                let tokens = self.tokens.clone();
                let control = cx.new(|cx| {
                    NativeControl::new(
                        ControlSpec {
                            kind: Kind::Push,
                            label: "Skip",
                            identifier: "stage-skip",
                            value: &serde_json::Value::Null,
                            options: vec![],
                            disabled: false,
                            default: false,
                            destructive: false,
                            inline: false,
                            quiet: true,
                            pull: false,
                            tint: None,
                            symbol: None,
                            glass: false,
                            well: false,
                        },
                        &tokens,
                        window,
                        cx,
                    )
                });
                cx.subscribe(
                    &control,
                    |stage: &mut Self, _, _: &serde_json::Value, cx| stage.skip(cx),
                )
                .detach();
                self.skip = Some(control);
            }
            self.skip.clone().unwrap().into_any_element()
        } else {
            // Headless windows have no AppKit: the same action, drawn.
            div()
                .id("stage-skip")
                .debug_selector(|| "stage-skip".into())
                .role(Role::Button)
                .aria_label("Skip")
                .text_size(px(self.tokens.typography.body))
                .text_color(rgba((self.tokens.colors.secondary << 8) | 0x90))
                .child("Skip")
                .on_click(cx.listener(|stage, _, _, cx| stage.skip(cx)))
                .into_any_element()
        }
    }
    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            self.skip(cx);
        }
    }
}

impl Render for Stage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.now();
        self.sounds(t, cx);
        // The shaders compile on workers from the first frame on.
        self.wave.update(cx, |w, _| w.warm(window));
        self.light.warm(window);
        let area = window.bounds().size;
        let scale = window.scale_factor();
        let phase = self.sheet.phase(t);
        let live = self.sheet.length();
        let stepping = self.step_springs(t, cx);
        let resting = t > live + 0.5 && !stepping && !self.curtain.fading();
        if self.clock.is_none() && !resting {
            window.request_animation_frame();
        }
        // The desktop dims from when the stage opens: a black veil over the
        // person's own windows (Reduce Transparency: the veil is solid). The
        // same veil under Reduce Motion; only the motion is a cross-fade.
        let opened = self.raw();
        let mut veil = if self.tokens.accessibility.reduce_transparency {
            self.sheet.dim.at(opened).min(1.) as f32
        } else {
            self.sheet.veil(opened, t) as f32
        };
        // What the stage shows comes up over its first moments (a launch
        // straight to the window too): the first frame is always clear.
        let mut stage_alpha =
            smooth(opened / if self.motion() { 0.35 } else { 0.6 }) as f32;
        // The lights go down once the Welcome has its own window, and with
        // the hand-off: the veil and the mark's light together.
        let lights = self.curtain.left(&self.sheet, self.motion(), t) as f32;
        stage_alpha *= lights;
        veil *= lights;
        if lights <= 0. {
            self.lights_out(false, cx);
        }
        let picture: AnyElement = if let Some(since) = self.curtain.out_since(t) {
            let out = smooth(since / WINDOW_OUT);
            if lights <= 0. {
                div().into_any_element()
            } else {
                self.set_snapshots(true, cx);
                self.window_scene(t, area, scale, out)
            }
        } else if phase == Phase::Window {
            // Live: the owner moves the Welcome into a real window (an app
            // with AppKit; captures and tests keep the drawn one).
            if !self.live_sent && self.clock.is_none() && crate::text::available(window) {
                self.live_sent = true;
                cx.emit(StageEvent::WindowLive);
            }
            if self.prepared {
                self.prepared = false;
                self.wave.update(cx, |w, _| w.clear());
                if self.leaving.is_none() {
                    self.set_snapshots(false, cx);
                }
            }
            let mut fade = 1.;
            if let Some(at) = self.skipped_at
                && t - at < SKIP_FADE
            {
                fade = smooth((t - at) / SKIP_FADE);
            }
            let scene = self.window_scene(t, area, scale, 0.);
            div()
                .size_full()
                .child(div().size_full().opacity(fade as f32).child(scene))
                .into_any_element()
        } else if t >= self.sheet.hold {
            // The hold: both pictures are taken now, on the GPU, before the
            // wave's first frame; then the wave draws the window.
            self.wave_scene(t, area, scale, window, cx)
        } else {
            self.intro(t, area, scale)
        };
        let title = self.sheet.title(phase);
        let skippable = t < live && !self.curtain.handing_off();
        let mut root = div()
            .id("stage")
            .debug_selector(|| "stage".into())
            .role(Role::Group)
            .accessibility_id("shell.stage")
            .aria_label("Welcome to Powermove")
            .relative()
            .size_full()
            .font_family(pm_design::TEXT_FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(rgba((veil * 255.).round() as u32)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(stage_alpha)
                    .child(picture),
            )
            .child(
                // VoiceOver hears each part's title as it starts.
                div()
                    .id("stage-scene-title")
                    .absolute()
                    .size_0()
                    .overflow_hidden()
                    .role(Role::Status)
                    .aria_live(gpui::accesskit::Live::Polite)
                    .aria_label(title),
            );
        // Built from the first frame (the stage is still clear then), so its
        // first layout never lands on a frame that moves.
        if skippable {
            let m = &self.tokens.metrics;
            root = root.child(
                div()
                    .absolute()
                    .right(px(m.sheet_inset * 1.5))
                    .bottom(px(m.sheet_inset * 1.5))
                    .opacity(stage_alpha)
                    .child(self.skip_button(window, cx)),
            );
        }
        root
    }
}

/// Opens the stage over the whole primary display: borderless and clear (the
/// person's desktop shows through, under the veil), never a white or
/// half-drawn window, above the menu bar and the Dock.
pub fn open_stage(
    build: impl FnOnce(&mut Window, &mut Context<Stage>) -> Stage + 'static,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Stage>> {
    let display = cx
        .primary_display()
        .ok_or_else(|| anyhow::anyhow!("no display for the stage"))?;
    let bounds = display.bounds();
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: Some(display.id()),
            macos_window_background: MacosWindowBackground::Transparent,
            ..Default::default()
        },
        |window, cx| {
            raise(window);
            let stage = cx.new(|cx| build(window, cx));
            stage.update(cx, |stage, cx| {
                window.focus(&stage.focus, cx);
                // A click on the room while the Welcome is in its own
                // window gives the keys back to that window.
                cx.observe_window_activation(window, |stage: &mut Stage, window, cx| {
                    if window.is_window_active()
                        && stage.native_drawn
                        && let Some(native) = stage.native
                    {
                        cx.defer(move |cx| {
                            let _ = native.update(cx, |_, window, _| window.activate());
                        });
                    }
                })
                .detach();
            });
            window.on_window_should_close(cx, |_, cx| {
                cx.quit();
                true
            });
            stage
        },
    )
}
/// Above the menu bar and the Dock, on every Space, without a shadow.
fn raise(window: &Window) {
    use objc2_app_kit::{NSColor, NSView, NSWindowCollectionBehavior};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(RawWindowHandle::AppKit(handle)) =
        HasWindowHandle::window_handle(window).map(|h| h.as_raw())
    else {
        return;
    };
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    let Some(ns) = view.window() else { return };
    // NSStatusWindowLevel: over the menu bar (24) and the Dock (20).
    ns.setLevel(25);
    // The whole screen, the menu bar's strip too (made at the normal level
    // it was kept under it; GPUI leaves a status-level frame as asked): the
    // veil dims all of it.
    if let Some(screen) = ns.screen() {
        ns.setFrame_display(screen.frame(), true);
    }
    ns.setHasShadow(false);
    ns.setOpaque(false);
    ns.setBackgroundColor(Some(&NSColor::clearColor()));
    ns.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary,
    );
}

/// `d`'s shadows at `k` of their strength.
fn faded_shadow<T: Styled>(mut d: T, k: f32) -> T {
    for s in d.style().box_shadow.iter_mut().flatten() {
        if let Some(c) = s.color.as_solid() {
            s.color = c.opacity(k).into();
        }
    }
    d
}

/// The snapshot a card's AppKit buttons paint while a wave or a step change
/// carries them.
pub fn snapshot_build(revision: u64) -> BuildSnapshot {
    BuildSnapshot {
        revision: Some(revision),
        ready: Some(0.),
        filter: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CueSheet, Curtain,
        cue::{LIGHTS_DOWN, LIGHTS_DOWN_REDUCED},
    };
    #[test]
    fn the_lights_go_down_once_the_welcome_has_its_own_window_then_the_stage_goes_once() {
        let sheet = CueSheet::default();
        let mut c = Curtain::default();
        let at = sheet.length() + 0.4;
        // The intro and the drawn window keep their dim and light.
        assert_eq!(c.left(&sheet, true, at), 1.);
        assert!(!c.fading());
        assert!(!c.take_dark(&sheet, true, at + 60., false));
        assert!(
            !c.take_dark(&sheet, true, at + 60., true),
            "no stage to clear before"
        );
        c.native_shown(at);
        assert!(c.fading());
        // Subtle and eased: most of it in the first half, none of it after.
        let half = c.left(&sheet, true, at + LIGHTS_DOWN / 2.);
        assert!(half > 0. && half < 0.5, "{half}");
        assert!(!c.take_dark(&sheet, true, at + 0.1, false));
        assert_eq!(c.left(&sheet, true, at + LIGHTS_DOWN), 0.);
        assert!(c.take_dark(&sheet, true, at + LIGHTS_DOWN, false));
        assert!(c.gone());
        // Gone stays gone: a later frame, the safety net or a hand-off never
        // bring it back or remove it twice.
        assert!(!c.take_dark(&sheet, true, at + 5., true));
        c.hand_off(at + 10.);
        assert_eq!(c.left(&sheet, true, at + 10.), 0.);
        assert!(!c.take_dark(&sheet, true, at + 20., false));
        // Showing again (a second `native_shown`) keeps the first start.
        c.native_shown(at + 30.);
        assert_eq!(c.left(&sheet, true, at + 30.), 0.);
    }
    #[test]
    fn reduce_motion_is_a_plain_quick_cross_fade() {
        let sheet = CueSheet::default();
        let mut c = Curtain::default();
        c.native_shown(0.);
        let half = c.left(&sheet, false, LIGHTS_DOWN_REDUCED / 2.);
        assert!((half - 0.5).abs() < 0.02, "linear: {half}");
        assert!(c.take_dark(&sheet, false, LIGHTS_DOWN_REDUCED, false));
    }
    #[test]
    fn a_hand_off_fades_what_is_left_and_the_replay_without_a_real_window_still_goes() {
        let sheet = CueSheet::default();
        // Mid-way down: the hand-off only takes it further, never back up.
        let mut c = Curtain::default();
        c.native_shown(0.);
        let before = c.left(&sheet, true, 0.2);
        c.hand_off(0.2);
        let mut last = before;
        for i in 0..=30 {
            let l = c.left(&sheet, true, 0.2 + f64::from(i) * 0.05);
            assert!(l <= last + 1e-9, "{l} after {last}");
            last = l;
        }
        assert!(c.take_dark(&sheet, true, 0.2 + sheet.dim_out.seconds, false));
        // A stage with no real window (captures, or one that never showed):
        // the hand-off alone takes it out, once; the safety net after it
        // does nothing.
        let mut c = Curtain::default();
        c.hand_off(9.);
        assert!(c.fading());
        assert!(c.left(&sheet, true, 9.1) > 0.);
        assert!(c.take_dark(&sheet, true, 9.02 + sheet.dim_out.seconds, false));
        assert!(!c.take_dark(&sheet, true, 30., true));
        // The safety net clears a stage that stopped drawing mid-fade.
        let mut c = Curtain::default();
        c.native_shown(0.);
        assert!(c.take_dark(&sheet, true, 0.1, true));
    }
}
