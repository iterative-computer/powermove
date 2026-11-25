//! The first-run stage: one borderless window over the whole display. The
//! desktop dims, a quiet credit shows, the Powermove mark rises from below
//! the bottom edge into the centre in its own light, and the distortion
//! wave draws the Welcome window out of that light. The first-run steps
//! play out in that window; the last one hands off into the app. Timing is
//! the cue sheet's (`cue`). The room's light is the engine's (`Pictures`).
//! Both pictures the wave carries are taken on the GPU while the intro
//! holds still, before the frame that needs them.
pub mod cue;
mod furniture;

use crate::native_control::{BuildSnapshot, ControlSpec, Kind, NativeControl};
use crate::wave::{DistortionWave, Palette, ReducedMotion, WaveRect, WaveState, WaveStyle};
use crate::welcome_view::WelcomeView;
pub use cue::{CueSheet, Phase};
use cue::{smooth, span};
use gpui::{prelude::*, *};
use pm_design::Tokens;
use std::{rc::Rc, sync::Arc, time::Instant};

/// The engine's side of the stage (the app's `stage_pictures`): the room.
pub trait Pictures {
    /// The newest picture of the room at `t` at `size` device pixels; it
    /// may be a frame behind while the next draws. `None` until one is ready.
    fn picture(&mut self, t: f64, size: (u32, u32)) -> Option<Arc<NativeTexture>>;
}
/// A sound cue (`cue::Sound::id`) reached.
pub type OnSound = Rc<dyn Fn(&str, &mut App)>;
/// What runs once the hand-off's lights are up (the stage closes).
type AfterHandOff = Box<dyn FnOnce(&mut Window, &mut App)>;

/// Seconds the room takes to fade in after a Skip, and out after the hand-off.
const SKIP_FADE: f64 = 0.35;
const LIGHTS_UP: f64 = 0.45;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WaveKey {
    Window,
    Step(u64),
    HandOff,
}
/// The app's first window, pictured on the GPU, and where it sits.
pub struct Arrival {
    pub picture: Arc<NativeTexture>,
    pub at: Bounds<Pixels>,
    /// The window's ground under its picture (its AppKit toolbar is not
    /// part of GPUI's picture).
    pub ground: u32,
    pub radius: f32,
}
struct HandOff {
    start: f64,
    arrival: Option<Arrival>,
    done: Option<AfterHandOff>,
}

pub struct Stage {
    pub sheet: CueSheet,
    pub tokens: Tokens,
    pictures: Box<dyn Pictures>,
    opened: Instant,
    /// Tests and recordings pin the clock (seconds).
    pub clock: Option<f64>,
    /// Added to the clock: a Skip moves the intro to the window.
    offset: f64,
    skipped_at: Option<f64>,
    card: Option<Entity<WelcomeView>>,
    /// The card as it was before a step changed it, for the step's wave.
    leaving_card: Option<(Entity<WelcomeView>, f64)>,
    steps: u64,
    wave: Entity<WaveState>,
    wave_for: Option<WaveKey>,
    /// The window's wave has its pictures (taken during the hold).
    window_prepared: bool,
    last_t: f64,
    on_sound: Option<OnSound>,
    focus: FocusHandle,
    skip: Option<Entity<NativeControl>>,
    hand_off: Option<HandOff>,
}

impl Stage {
    /// `intro` false: the window at once (a launch with nothing to welcome).
    pub fn new(
        tokens: Tokens,
        sheet: CueSheet,
        pictures: Box<dyn Pictures>,
        intro: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let wave = cx.new(|_| {
            WaveState::new(
                wave_style(pm_wave::WaveType::Center, &tokens),
                WaveRect {
                    x: 0.,
                    y: 0.,
                    w: 1.,
                    h: 1.,
                },
            )
            .expect("the stage's wave style is valid")
        });
        let offset = if intro { 0. } else { sheet.length() };
        Self {
            sheet,
            tokens,
            pictures,
            opened: Instant::now(),
            clock: None,
            offset,
            skipped_at: None,
            card: None,
            leaving_card: None,
            steps: 0,
            wave,
            wave_for: None,
            window_prepared: false,
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
    /// The Welcome window's content; a later step arrives with a wave from
    /// the one before.
    pub fn set_card(&mut self, card: Entity<WelcomeView>, cx: &mut Context<Self>) {
        let t = self.now();
        if let Some(previous) = self.card.replace(card)
            && t >= self.sheet.length()
        {
            self.steps += 1;
            self.leaving_card = Some((previous, t));
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
        self.release_wave(cx);
        cx.notify();
    }
    /// The last step: the wave carries the window into the app's first
    /// window as drawn (`arrival`), then the lights come up on the real one
    /// and `done` runs.
    pub fn hand_off(
        &mut self,
        arrival: Option<Arrival>,
        done: impl FnOnce(&mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let start = self.now().max(self.sheet.length());
        self.offset += start - self.now();
        self.hand_off = Some(HandOff {
            start,
            arrival,
            done: Some(Box::new(done)),
        });
        cx.notify();
    }
    fn release_wave(&mut self, cx: &mut Context<Self>) {
        if self.wave_for.take().is_some() {
            self.wave.update(cx, |w, _| w.clear());
        }
        self.window_prepared = false;
    }
    fn motion(&self) -> bool {
        !self.tokens.accessibility.reduce_motion
    }
    /// The black veil over the desktop, from when the stage opens.
    fn veil(&self, opened: f64) -> f32 {
        if self.tokens.accessibility.reduce_transparency {
            return 1.;
        }
        (0.94 * span(opened, 0., self.sheet.dimmed)) as f32
    }
    /// The room fades up once the house lights are down.
    fn room(&self, opened: f64) -> f32 {
        let lit = self.sheet.dimmed * 0.6;
        span(opened, lit, lit + 0.8) as f32
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
    /// The room's light at `t` (its grain at film's 24 frames a second;
    /// still from the hold on).
    fn room_picture(&mut self, t: f64, area: Size<Pixels>, scale: f32) -> AnyElement {
        let t = (t.min(self.sheet.hold) * 24.).floor() / 24.;
        let size = (
            ((f32::from(area.width) * scale * 0.5).round() as u32).max(1),
            ((f32::from(area.height) * scale * 0.5).round() as u32).max(1),
        );
        texture_fill(self.pictures.picture(t, size)).into_any_element()
    }
    /// The intro at `t`: the room, the credit, the mark in its light.
    fn intro(&mut self, t: f64, area: Size<Pixels>, scale: f32) -> AnyElement {
        let t = t.min(self.sheet.hold);
        let motion = self.motion();
        div()
            .id("stage-intro")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x000000))
            .child(self.room_picture(t, area, scale))
            .child(furniture::credit(t, &self.sheet, &self.tokens))
            .child(furniture::mark(t, &self.sheet, &self.tokens, motion))
            .into_any_element()
    }
    /// The Welcome window standing on the room.
    fn window_scene(&mut self, area: Size<Pixels>, scale: f32) -> AnyElement {
        let hold = self.sheet.hold;
        let mut root = div()
            .id("stage-window")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x000000))
            .child(self.room_picture(hold, area, scale));
        if let Some(card) = &self.card {
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(card.clone()),
            );
        }
        root.into_any_element()
    }
    /// The room with the app's first window drawn where it sits: where the
    /// hand-off's wave arrives.
    fn arrival(&mut self, area: Size<Pixels>, scale: f32) -> AnyElement {
        let hold = self.sheet.hold;
        let mut room = div()
            .relative()
            .size_full()
            .bg(rgb(0x000000))
            .child(self.room_picture(hold, area, scale));
        if let Some(arrival) = self.hand_off.as_ref().and_then(|h| h.arrival.as_ref()) {
            let (picture, at, ground, radius) = (
                arrival.picture.clone(),
                arrival.at,
                arrival.ground,
                px(arrival.radius),
            );
            room = room.child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        window.paint_quad(
                            PaintQuad::new(at)
                                .corner_radii(Corners::all(radius))
                                .background(rgb(ground)),
                        );
                        window.paint_texture(at, picture.clone());
                    },
                )
                .absolute()
                .inset_0(),
            );
        }
        room.into_any_element()
    }
    /// Points the wave at a new transition (`key`) from `source`.
    fn arm_wave(
        &mut self,
        key: WaveKey,
        source: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.wave_for == Some(key) {
            return;
        }
        let style = wave_style(pm_wave::WaveType::Center, &self.tokens);
        self.wave.update(cx, |w, _| {
            w.clear();
            let _ = w.set_style(style, window);
            let _ = w.set_source(WaveRect {
                x: f64::from(source.origin.x),
                y: f64::from(source.origin.y),
                w: f64::from(source.size.width).max(1.),
                h: f64::from(source.size.height).max(1.),
            });
            w.set_reduced_motion(ReducedMotion::System, window);
        });
        self.wave_for = Some(key);
        self.window_prepared = false;
    }
    fn waved(
        &mut self,
        p: f64,
        from: AnyElement,
        to: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = self.wave.update(cx, |w, _| w.seek(p.max(1e-4)));
        DistortionWave::new(self.wave.clone(), from)
            .revealed_content(to)
            .size_full()
            .into_any_element()
    }
    /// The mark's place, where the window's wave starts.
    fn mark_rect(&self, area: Size<Pixels>) -> Bounds<Pixels> {
        let (cx, cy, height) = furniture::mark_at(self.sheet.rise.1, &self.sheet, self.motion());
        let h = f32::from(area.height) * height as f32;
        Bounds::new(
            point(
                px(f32::from(area.width) * cx as f32 - h / 2.),
                px(f32::from(area.height) * cy as f32 - h / 2.),
            ),
            size(px(h), px(h)),
        )
    }
    fn set_card_snapshot(&self, on: bool, cx: &mut Context<Self>) {
        if let Some(card) = &self.card {
            card.update(cx, |c, cx| c.set_snapshot(on, cx));
        }
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
                .text_color(rgba((self.tokens.colors.secondary << 8) | 0xb0))
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

/// The wave the stage uses: Powermove's accent on the rim, square corners
/// (the picture is the whole display).
/// The stage's wave is the mark's light, quiet: a soft warm rim and
/// hairline, no sparks.
fn wave_style(kind: pm_wave::WaveType, tokens: &Tokens) -> WaveStyle {
    let a = furniture::glow(tokens);
    let base = WaveStyle::new(kind);
    WaveStyle {
        palette: Palette::Single([(a >> 16) as u8, (a >> 8) as u8, a as u8]),
        corner_radius: 0.,
        rim_strength: base.rim_strength * 0.5,
        hairline_opacity: base.hairline_opacity * 0.4,
        particle_count: 0.,
        ..base
    }
}

/// A picture filling its parent, or nothing until it is ready.
fn texture_fill(texture: Option<Arc<NativeTexture>>) -> impl IntoElement {
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
}

impl Render for Stage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.now();
        self.sounds(t, cx);
        // The wave's shaders compile on a worker from the first frame on.
        self.wave.update(cx, |w, _| w.warm(window));
        let area = window.bounds().size;
        let scale = window.scale_factor();
        let phase = self.sheet.phase(t);
        let interactive = self.sheet.length();
        let resting =
            t > interactive + 0.5 && self.leaving_card.is_none() && self.hand_off.is_none();
        if !resting && self.clock.is_none() {
            window.request_animation_frame();
        }
        // The house lights go down from when the stage opens.
        let opened = self.raw();
        let mut stage_alpha = self.room(opened);
        let mut veil = self.veil(opened);
        let picture: AnyElement = if self.hand_off.is_some() {
            let start = self.hand_off.as_ref().unwrap().start;
            let p = ((t - start) / self.sheet.wave_seconds).clamp(0., 1.);
            let up = ((t - start - self.sheet.wave_seconds) / LIGHTS_UP).clamp(0., 1.);
            stage_alpha = 1. - smooth(up) as f32;
            veil *= stage_alpha;
            if up >= 1. {
                if let Some(done) = self.hand_off.as_mut().and_then(|h| h.done.take()) {
                    cx.defer_in(window, move |_, window, cx| done(window, cx));
                }
                div().into_any_element()
            } else if p < 1. {
                let source = self
                    .hand_off
                    .as_ref()
                    .and_then(|h| h.arrival.as_ref().map(|a| a.at))
                    .unwrap_or_else(|| Bounds::new(point(px(0.), px(0.)), area));
                self.arm_wave(WaveKey::HandOff, source, window, cx);
                self.set_card_snapshot(true, cx);
                let from = self.window_scene(area, scale);
                let to = self.arrival(area, scale);
                self.waved(p, from, to, cx)
            } else {
                self.release_wave(cx);
                self.arrival(area, scale)
            }
        } else if matches!(phase, Phase::Spread | Phase::Wave) && t >= self.sheet.hold {
            // The hold: both pictures are taken now, on the GPU, before the
            // wave's first frame; then the wave draws the window.
            let source = self.mark_rect(area);
            self.arm_wave(WaveKey::Window, source, window, cx);
            self.set_card_snapshot(true, cx);
            if !self.window_prepared {
                self.window_prepared = true;
                self.wave.update(cx, |w, _| w.prepare_now());
            }
            let p = (t - self.sheet.window) / self.sheet.wave_seconds;
            if p > 0. {
                let _ = self.wave.update(cx, |w, _| w.seek(p.min(1. - 1e-4)));
            }
            let from = self.intro(t, area, scale);
            let to = self.window_scene(area, scale);
            DistortionWave::new(self.wave.clone(), from)
                .revealed_content(to)
                .size_full()
                .into_any_element()
        } else if let Some((previous, began)) = self.leaving_card.clone() {
            // A step in the window: the wave from the step before.
            let p = (t - began) / self.sheet.wave_seconds;
            if p >= 1. {
                self.leaving_card = None;
                self.release_wave(cx);
                self.set_card_snapshot(false, cx);
                self.window_scene(area, scale)
            } else {
                let key = WaveKey::Step(self.steps);
                let full = Bounds::new(point(px(0.), px(0.)), area);
                self.arm_wave(key, full, window, cx);
                previous.update(cx, |c, cx| c.set_snapshot(true, cx));
                self.set_card_snapshot(true, cx);
                let current = self.card.replace(previous);
                let from = self.window_scene(area, scale);
                self.card = current;
                let to = self.window_scene(area, scale);
                self.waved(p, from, to, cx)
            }
        } else if phase == Phase::Window {
            self.release_wave(cx);
            self.set_card_snapshot(false, cx);
            let mut fade = 1.;
            if let Some(at) = self.skipped_at
                && t - at < SKIP_FADE
            {
                fade = smooth((t - at) / SKIP_FADE);
            }
            let scene = self.window_scene(area, scale);
            div()
                .size_full()
                .bg(rgb(0x000000))
                .child(div().size_full().opacity(fade as f32).child(scene))
                .into_any_element()
        } else {
            self.intro(t, area, scale)
        };
        let title = self.sheet.title(phase);
        let skippable = t < interactive && self.hand_off.is_none();
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

/// The snapshot a card's AppKit buttons paint while a wave carries them.
pub fn snapshot_build(revision: u64) -> BuildSnapshot {
    BuildSnapshot {
        revision: Some(revision),
        ready: Some(0.),
        filter: None,
    }
}
