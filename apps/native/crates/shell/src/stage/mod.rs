//! The first-run stage: one borderless window over the whole display. The
//! desktop dims, a quiet credit shows, the Powermove mark rises from below
//! the bottom edge into the centre as a lit object, and the distortion wave
//! draws the Welcome window out of the mark's light, once, on the window's
//! own rectangle. The first-run steps change inside the window with a quick
//! spring cross-fade; the last one hands off into the app: the window eases
//! out while the room's lights come up. Timing is the cue sheet's (`cue`),
//! Dia's opening measured; the room and the mark are `light`'s one pass.
pub mod cue;
pub mod intro;
pub mod light;

use crate::kit::Recipes;
use crate::native_control::{BuildSnapshot, ControlSpec, Kind, NativeControl};
use crate::wave::{DistortionWave, ReducedMotion, WaveRect, WaveState};
use crate::welcome_view::WelcomeView;
pub use cue::{CueSheet, Phase};
use cue::smooth;
use gpui::{prelude::*, *};
use pm_design::{
    Elevation, Tokens,
    motion::{Animated, Spring, UNIT_PRECISION},
};
use std::{rc::Rc, time::Instant};

/// A sound cue (`cue::Sound::id`) reached.
pub type OnSound = Rc<dyn Fn(&str, &mut App)>;
/// What runs once the hand-off's lights are up (the stage closes).
type AfterHandOff = Box<dyn FnOnce(&mut Window, &mut App)>;

/// Seconds the room takes to fade in after a Skip.
const SKIP_FADE: f64 = 0.35;
/// The hand-off: the window eases out while the lights come up.
const WINDOW_OUT: f64 = 0.32;
const LIGHTS_UP: f64 = 0.55;
/// A step's content arrives from a few points below and leaves a few up.
const SLIDE_IN: f32 = 10.;
const SLIDE_OUT: f32 = 6.;
/// quiet-md's quick fade (critically damped) and its vanish.
/// The mark in the Welcome window: its height (points) and centre (a
/// fraction of the window's height).
pub const WINDOW_MARK: f32 = 84.;
pub const WINDOW_MARK_Y: f32 = 0.31;
const ARRIVE: Spring = Spring::new(520., 46.);
const LEAVE: Spring = Spring::new(900., 60.);

struct HandOff {
    start: f64,
    done: Option<AfterHandOff>,
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
    hand_off: Option<HandOff>,
}

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
            hand_off: None,
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
    /// The last step: the window eases out while the room's lights come up
    /// on the app's first window behind the stage, then `done` runs.
    pub fn hand_off(&mut self, done: impl FnOnce(&mut Window, &mut App) + 'static, cx: &mut Context<Self>) {
        let start = self.now().max(self.sheet.length());
        self.offset += start - self.now();
        self.hand_off = Some(HandOff {
            start,
            done: Some(Box::new(done)),
        });
        cx.notify();
    }
    fn motion(&self) -> bool {
        !self.tokens.accessibility.reduce_motion
    }
    /// How far the house lights are down (0–1), from when the stage opens:
    /// Dia's room light going down.
    fn dimmed(&self, opened: f64) -> f64 {
        if self.motion() {
            self.sheet.dim.at(opened)
        } else {
            smooth(opened / 0.6)
        }
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
    /// The room and the mark at `t`, filling the stage.
    fn room(&mut self, t: f64, area: Size<Pixels>, scale: f32) -> AnyElement {
        let size = (
            (f32::from(area.width) * scale).round(),
            (f32::from(area.height) * scale).round(),
        );
        let light = intro::light_at(t, &self.sheet, self.motion(), size, scale);
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
    /// The intro at `t`: the room and the mark, the credit, the wordmark.
    fn intro(&mut self, t: f64, area: Size<Pixels>, scale: f32) -> AnyElement {
        let motion = self.motion();
        div()
            .id("stage-intro")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x000000))
            .child(self.room(t, area, scale))
            .child(intro::credit(t, &self.sheet, area))
            .child(intro::words(t, &self.sheet, area, &self.tokens, motion))
            .into_any_element()
    }
    /// The Welcome window's rectangle on the stage: centred.
    fn window_rect(&self, area: Size<Pixels>) -> Bounds<Pixels> {
        let m = &self.tokens.metrics;
        let size = size(px(m.welcome_width), px(m.welcome_height));
        Bounds::new(
            point((area.width - size.width) / 2., (area.height - size.height) / 2.),
            size,
        )
    }
    fn window_radius(&self) -> f32 {
        let m = &self.tokens.metrics;
        m.group_radius + m.sheet_inset
    }
    /// The window: its ground and the current step's content, with a
    /// leaving step's content fading over it.
    /// The window's ground, lit by the same pass as the room: a light from
    /// its top, and on the Welcome step the mark in it (still: the window
    /// at rest draws no frames).
    fn window_light(&mut self, rect: Bounds<Pixels>, scale: f32, cx: &App) -> Option<std::sync::Arc<NativeTexture>> {
        let (w, h) = (
            (f32::from(rect.size.width) * scale).round(),
            (f32::from(rect.size.height) * scale).round(),
        );
        let welcome = self
            .card
            .as_ref()
            .is_some_and(|c| c.read(cx).frame.animate);
        let mark = WINDOW_MARK * scale;
        let ground = self.tokens.colors.window;
        let linear = |shift: u32| {
            let v = ((ground >> shift) & 255) as f32 / 255.;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        let light = light::Light {
            size: [w, h],
            centre: if welcome { [w / 2., h * WINDOW_MARK_Y] } else { [w / 2., -0.3 * h] },
            scale: mark / 112.,
            exposure: 1.,
            bloom: if welcome { 0.5 } else { 0. },
            edge: 0.,
            pool: if welcome { 0.7 } else { 0.55 },
            mark: if welcome { 1. } else { 0. },
            time: self.sheet.hold as f32,
            density: scale,
            reach: if welcome { mark * 1.2 } else { 0.45 * h },
            ground: [linear(16), linear(8), linear(0)],
            domain: [0.; 4],
        };
        self.light.picture(light, self.wait)
    }
    fn window(&self, rect: Bounds<Pixels>, shadow: f32, lit: Option<std::sync::Arc<NativeTexture>>) -> Stateful<Div> {
        let t = &self.tokens;
        let mut ground = div()
            .id("stage-window")
            .debug_selector(|| "stage-window".into())
            .relative()
            .w(rect.size.width)
            .h(rect.size.height)
            .corner(t, self.window_radius())
            .bg(rgb(t.colors.window))
            .border_1()
            .border_color(rgba((t.colors.text << 8) | 0x16))
            .overflow_hidden()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        if let Some(lit) = lit {
                            window.paint_texture(bounds, lit);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            );
        if shadow > 0. {
            // The shadow arrives with the window, not with its picture.
            ground = faded_shadow(ground.elevated(t, Elevation::Raised), shadow);
        }
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
    /// The window standing in the room at `t`.
    fn window_scene(&mut self, t: f64, area: Size<Pixels>, scale: f32, out: f64, cx: &App) -> AnyElement {
        let rect = self.window_rect(area);
        let lit = self.window_light(rect, scale, cx);
        // Out: the window eases away, settling a few points as it goes.
        let w = self
            .window(rect, 1., lit)
            .top(px(8. * out as f32))
            .opacity(1. - out as f32);
        div()
            .relative()
            .size_full()
            .bg(rgb(0x000000))
            .child(self.room(t, area, scale))
            .child(
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
    fn wave_scene(&mut self, t: f64, area: Size<Pixels>, scale: f32, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let hold = self.sheet.hold;
        let rect = self.window_rect(area);
        if !self.prepared {
            self.prepared = true;
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
                w.set_reduced_motion(ReducedMotion::System, window);
                w.prepare_now();
            });
            if let Some(card) = &self.card {
                card.update(cx, |c, cx| c.set_snapshot(true, cx));
            }
        }
        let p = (t - self.sheet.window) / self.sheet.wave_seconds;
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
        let lit = self.window_light(rect, scale, cx);
        let to = self.window(rect, 0., lit);
        let shadow = smooth((p - 0.2) / 0.3) as f32;
        let mut stage = div()
            .relative()
            .size_full()
            .child(self.intro(hold, area, scale));
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
        for card in self.card.iter().chain(self.leaving.as_ref().map(|l| &l.card)) {
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
        let resting = t > live + 0.5 && !stepping && self.hand_off.is_none();
        if self.clock.is_none() && !resting {
            window.request_animation_frame();
        }
        // The house lights go down from when the stage opens.
        let opened = self.raw();
        let dimmed = self.dimmed(opened) as f32;
        let mut veil = if self.tokens.accessibility.reduce_transparency {
            1.
        } else {
            0.94 * dimmed
        };
        let mut stage_alpha = dimmed;
        let picture: AnyElement = if let Some(hand_off) = self.hand_off.as_mut() {
            let since = t - hand_off.start;
            let out = smooth(since / WINDOW_OUT);
            let up = smooth((since - WINDOW_OUT * 0.4) / LIGHTS_UP);
            stage_alpha *= 1. - up as f32;
            veil *= 1. - up as f32;
            if up >= 1. {
                if let Some(done) = hand_off.done.take() {
                    cx.defer_in(window, move |_, window, cx| done(window, cx));
                }
                div().into_any_element()
            } else {
                self.set_snapshots(true, cx);
                self.window_scene(t, area, scale, out, cx)
            }
        } else if phase == Phase::Window {
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
            let scene = self.window_scene(t, area, scale, 0., cx);
            div()
                .size_full()
                .bg(rgb(0x000000))
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
        let skippable = t < live && self.hand_off.is_none();
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
        if skippable && stage_alpha > 0. {
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

/// Opens the stage over the whole primary display: borderless, clear until
/// its first frame draws the veil (never a white or half-drawn window),
/// above the menu bar and the Dock.
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
            stage.update(cx, |stage, cx| window.focus(&stage.focus, cx));
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
