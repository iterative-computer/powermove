//! The first-run stage: one borderless window over the whole display that
//! dims the desktop, runs the film on the cue sheet (`cue`), and ends on the
//! Welcome card, where the first-run steps play out. Pictures (the room's
//! light and the reels) are the engine's, painted as GPUI textures through
//! `Pictures`, so the distortion wave carries the real rendered scene from
//! one cue into the next. The card's buttons are AppKit's; during a wave
//! they paint their own snapshot so the wave distorts them too.
pub mod beats;
pub mod cue;
mod furniture;

use crate::native_control::{BuildSnapshot, ControlSpec, Kind, NativeControl};
use crate::wave::{DistortionWave, Palette, ReducedMotion, WaveRect, WaveState, WaveStyle};
use crate::welcome_view::WelcomeView;
pub use cue::{CueSheet, Reel, Scene, Transition};
use gpui::{prelude::*, *};
use pm_design::Tokens;
use std::{rc::Rc, sync::Arc, time::Instant};

/// A picture the engine draws for the stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Picture {
    /// The room: its light, grain and falloff, the whole display.
    Backdrop,
    Reel(Reel),
}
/// The engine's side of the stage (the app's `stage_pictures`).
pub trait Pictures {
    /// The newest picture of `what` at `t` (seconds into it) at `size`
    /// device pixels; it may be a frame behind while the next draws.
    /// `None` until a first one is ready.
    fn picture(&mut self, what: Picture, t: f64, size: (u32, u32)) -> Option<Arc<NativeTexture>>;
    /// The film needs no more pictures: the engine may let go.
    fn rest(&mut self) {}
}
/// A sound cue (`cue::Sound::id`) reached.
pub type OnSound = Rc<dyn Fn(&str, &mut App)>;

/// Seconds the room takes to fade in after a Skip, and out after the hand-off.
const SKIP_FADE: f64 = 0.35;
const LIGHTS_UP: f64 = 0.45;
/// How long before a wave the next scene's first picture is asked for.
const PREROLL: f64 = 0.5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WaveKey {
    Cue(usize),
    Step(u64),
    HandOff,
}
struct HandOff {
    start: f64,
    /// The app's first window as drawn, where it sits on the display.
    picture: Option<(Arc<RenderImage>, Bounds<Pixels>)>,
    done: Option<Box<dyn FnOnce(&mut Window, &mut App)>>,
}

pub struct Stage {
    pub sheet: CueSheet,
    pub tokens: Tokens,
    pictures: Box<dyn Pictures>,
    opened: Instant,
    /// Tests and recordings pin the film's clock (seconds).
    pub clock: Option<f64>,
    /// Added to the clock: a Skip moves the film to the card.
    offset: f64,
    skipped_at: Option<f64>,
    card: Option<Entity<WelcomeView>>,
    /// The card as it was before a step changed it, for the step's wave.
    leaving_card: Option<(Entity<WelcomeView>, f64)>,
    wave: Entity<WaveState>,
    /// Which transition the wave holds.
    wave_for: Option<WaveKey>,
    /// Counts the card's steps, so each step's wave is armed once.
    steps: u64,
    last_t: f64,
    on_sound: Option<OnSound>,
    focus: FocusHandle,
    skip: Option<Entity<NativeControl>>,
    on_skip: Option<Rc<dyn Fn(&mut App)>>,
    hand_off: Option<HandOff>,
    /// The scene VoiceOver last heard.
    spoken: Option<usize>,
}

impl Stage {
    pub fn new(
        tokens: Tokens,
        sheet: CueSheet,
        pictures: Box<dyn Pictures>,
        film: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let wave = cx.new(|_| {
            WaveState::new(
                wave_style(cue::STEP_WAVE, &tokens),
                WaveRect {
                    x: 0.,
                    y: 0.,
                    w: 1.,
                    h: 1.,
                },
            )
            .expect("the stage's wave style is valid")
        });
        let offset = if film { 0. } else { sheet.length() };
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
            wave,
            wave_for: None,
            steps: 0,
            last_t: -1.,
            on_sound: None,
            focus: cx.focus_handle(),
            skip: None,
            on_skip: None,
            hand_off: None,
            spoken: None,
        }
    }
    pub fn on_sound(&mut self, sound: OnSound) {
        self.on_sound = Some(sound);
    }
    /// Called when the person skips the film (the card then shows at once).
    pub fn on_skip(&mut self, skip: Rc<dyn Fn(&mut App)>) {
        self.on_skip = Some(skip);
    }
    /// The film's time: seconds since the stage opened, moved on by Skip.
    pub fn now(&self) -> f64 {
        self.clock
            .unwrap_or_else(|| self.opened.elapsed().as_secs_f64())
            + self.offset
    }
    pub fn card(&self) -> Option<&Entity<WelcomeView>> {
        self.card.as_ref()
    }
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }
    /// The Welcome card the film ends on; a later card (the next step)
    /// arrives with a wave from the one before.
    pub fn set_card(&mut self, card: Entity<WelcomeView>, cx: &mut Context<Self>) {
        let t = self.now();
        if let Some(previous) = self.card.replace(card)
            && self.on_card(t)
        {
            self.steps += 1;
            self.leaving_card = Some((previous, t));
        }
        cx.notify();
    }
    /// Whether the film is over and the card is the scene.
    pub fn on_card(&self, t: f64) -> bool {
        self.sheet.at(t).scene == Scene::Card
    }
    /// Esc or Skip: straight to the card, with a short fade from black.
    pub fn skip(&mut self, cx: &mut Context<Self>) {
        let t = self.now();
        let card = self.sheet.length();
        if t >= card {
            return;
        }
        self.offset += card - t;
        self.skipped_at = Some(card);
        self.release_wave(cx);
        if let Some(skip) = &self.on_skip {
            let skip = skip.clone();
            cx.defer(move |cx| skip(cx));
        }
        cx.notify();
    }
    /// The last step: a wave from the card into the app's first window as
    /// it was drawn (`picture`, where it sits), then the room's lights come
    /// up on the real window and `done` runs.
    pub fn hand_off(
        &mut self,
        picture: Option<(Arc<RenderImage>, Bounds<Pixels>)>,
        done: impl FnOnce(&mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let start = self.now().max(self.sheet.length());
        self.offset += start - self.now();
        self.hand_off = Some(HandOff {
            start,
            picture,
            done: Some(Box::new(done)),
        });
        cx.notify();
    }
    fn release_wave(&mut self, cx: &mut Context<Self>) {
        if self.wave_for.take().is_some() {
            self.wave.update(cx, |w, _| w.clear());
        }
    }
    fn reduce_motion(&self) -> bool {
        self.tokens.accessibility.reduce_motion
    }
    /// The black veil over the desktop: up first, before any picture.
    fn veil(&self, t: f64) -> f32 {
        if self.tokens.accessibility.reduce_transparency {
            return 1.;
        }
        let dim = beats::span(t, 0., self.sheet.cues[1].at);
        (0.94 * dim) as f32
    }
    /// The room itself fades up over the veil once the house lights are down.
    fn room(&self, t: f64) -> f32 {
        let lit = self.sheet.cues[1].at;
        beats::span(t, lit, lit + 0.8) as f32
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
    /// The picture rectangle the reels play on: 16:9, a little above centre.
    pub fn screen(bounds: Size<Pixels>) -> Bounds<Pixels> {
        let (w, h) = (f32::from(bounds.width), f32::from(bounds.height));
        let width = (w * 0.62).min(h * 0.56 * 16. / 9.);
        let height = width * 9. / 16.;
        Bounds::new(
            point(px((w - width) / 2.), px((h - height) / 2. - h * 0.035)),
            size(px(width), px(height)),
        )
    }
    fn device(size: Size<Pixels>, scale: f32, k: f32) -> (u32, u32) {
        (
            ((f32::from(size.width) * scale * k).round() as u32).max(1),
            ((f32::from(size.height) * scale * k).round() as u32).max(1),
        )
    }
    /// One scene as a picture at `local` seconds into it.
    fn scene(
        &mut self,
        index: usize,
        local: f64,
        t: f64,
        area: Size<Pixels>,
        scale: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scene = self.sheet.cues[index].scene;
        // The room's grain moves at film's 24 frames a second, which also
        // leaves the engine most display frames for the reels.
        let room_t = (t.min(self.sheet.length() + 1.5) * 24.).floor() / 24.;
        let backdrop =
            self.pictures
                .picture(Picture::Backdrop, room_t, Self::device(area, scale, 0.5));
        let mut root = div()
            .id(("stage-scene", index))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x000000))
            .child(texture_fill(backdrop));
        let t_ = self.tokens.clone();
        let motion = !self.reduce_motion();
        root = match scene {
            Scene::HouseLights => root,
            Scene::TitleCard => root.child(furniture::slate(local, &t_, motion)),
            Scene::Mark => root.child(furniture::mark(local, &t_, motion)),
            Scene::Reel(reel) => {
                let screen = Self::screen(area);
                let picture = self.pictures.picture(
                    Picture::Reel(reel),
                    local.min(beats::REEL_SECONDS),
                    Self::device(screen.size, scale, 1.),
                );
                root.child(furniture::reel(reel, local, screen, picture, &t_, window))
            }
            Scene::Card => match &self.card {
                Some(card) => root.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(card.clone()),
                ),
                None => root,
            },
        };
        let _ = cx;
        root.into_any_element()
    }
    /// The room with the app's first window drawn where it sits: where the
    /// hand-off's wave arrives.
    fn arrival(&self, hand_off: &HandOff) -> AnyElement {
        let mut room = div().relative().size_full().bg(rgb(0x000000));
        if let Some((image, at)) = &hand_off.picture {
            let (image, at) = (image.clone(), *at);
            room = room.child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        let _ = window.paint_image(
                            at,
                            at,
                            Corners::all(px(0.)),
                            0.,
                            image.clone(),
                            0,
                            false,
                        );
                    },
                )
                .absolute()
                .inset_0(),
            );
        }
        room.into_any_element()
    }
    /// Points the wave at a new transition (`key`) of `kind`, from `source`.
    fn arm_wave(
        &mut self,
        key: WaveKey,
        kind: pm_wave::WaveType,
        source: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.wave_for == Some(key) {
            return;
        }
        let style = wave_style(kind, &self.tokens);
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
fn wave_style(kind: pm_wave::WaveType, tokens: &Tokens) -> WaveStyle {
    let a = tokens.colors.accent;
    WaveStyle {
        palette: Palette::Single([(a >> 16) as u8, (a >> 8) as u8, a as u8]),
        corner_radius: 0.,
        ..WaveStyle::new(kind)
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
        let area = window.bounds().size;
        let scale = window.scale_factor();
        let position = self.sheet.at(t);
        let card_at = self.sheet.length();
        let resting = position.scene == Scene::Card
            && t > card_at + 2.
            && self.leaving_card.is_none()
            && self.hand_off.is_none();
        if resting {
            self.pictures.rest();
        } else if self.clock.is_none() {
            window.request_animation_frame();
        }
        // The next scene's first picture, ahead of the wave that shows it.
        if let Some(next) = self.sheet.cues.get(position.index + 1)
            && let Scene::Reel(reel) = next.scene
            && position.length - position.local < PREROLL + self.sheet.wave_seconds
        {
            let screen = Self::screen(area);
            let _ = self.pictures.picture(
                Picture::Reel(reel),
                0.,
                Self::device(screen.size, scale, 1.),
            );
        }
        let mut stage_alpha = self.room(t);
        let mut veil = self.veil(t);
        let picture: AnyElement = if let Some(hand_off) = self.hand_off.as_ref() {
            let p = ((t - hand_off.start) / self.sheet.wave_seconds).clamp(0., 1.);
            let up = ((t - hand_off.start - self.sheet.wave_seconds) / LIGHTS_UP).clamp(0., 1.);
            stage_alpha = 1. - beats::smooth(up) as f32;
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
                    .and_then(|h| h.picture.as_ref().map(|(_, at)| *at))
                    .unwrap_or_else(|| Self::screen(area));
                self.arm_wave(WaveKey::HandOff, cue::STEP_WAVE, source, window, cx);
                let _ = self.wave.update(cx, |w, _| w.seek(p));
                let card = self.sheet.cues.len() - 1;
                if let Some(card) = &self.card {
                    card.update(cx, |c, cx| c.set_snapshot(true, cx));
                }
                let from = self.scene(card, t - card_at, t, area, scale, window, cx);
                let to = self.arrival(self.hand_off.as_ref().unwrap());
                DistortionWave::new(self.wave.clone(), from)
                    .revealed_content(to)
                    .size_full()
                    .into_any_element()
            } else {
                self.release_wave(cx);
                self.arrival(self.hand_off.as_ref().unwrap())
            }
        } else if let Some((Transition::Wave(kind), p)) = position.leaving
            && p > 0.
        {
            let next = position.index + 1;
            let source = match self.sheet.cues[position.index].scene {
                Scene::Reel(_) | Scene::Mark => Self::screen(area),
                _ => Bounds::new(point(px(0.), px(0.)), area),
            };
            self.arm_wave(WaveKey::Cue(position.index), kind, source, window, cx);
            let _ = self.wave.update(cx, |w, _| w.seek(p));
            let started = position.length - self.sheet.wave_seconds;
            if self.sheet.cues[next].scene == Scene::Card
                && let Some(card) = &self.card
            {
                card.update(cx, |c, cx| c.set_snapshot(true, cx));
            }
            let from = self.scene(position.index, started, t, area, scale, window, cx);
            let to = self.scene(next, 0., t, area, scale, window, cx);
            DistortionWave::new(self.wave.clone(), from)
                .revealed_content(to)
                .size_full()
                .into_any_element()
        } else if let Some((previous, began)) = self.leaving_card.clone() {
            // A step on the card: the wave from the card before.
            let p = (t - began) / self.sheet.wave_seconds;
            let card = self.sheet.cues.len() - 1;
            if p >= 1. {
                self.leaving_card = None;
                self.release_wave(cx);
                if let Some(card) = &self.card {
                    card.update(cx, |c, cx| c.set_snapshot(false, cx));
                }
                self.scene(card, t - card_at, t, area, scale, window, cx)
            } else {
                self.arm_wave(
                    WaveKey::Step(self.steps),
                    cue::STEP_WAVE,
                    Self::screen(area),
                    window,
                    cx,
                );
                let _ = self.wave.update(cx, |w, _| w.seek(p.max(1e-4)));
                previous.update(cx, |c, cx| c.set_snapshot(true, cx));
                if let Some(card) = &self.card {
                    card.update(cx, |c, cx| c.set_snapshot(true, cx));
                }
                let current = self.card.replace(previous);
                let from = self.scene(card, t - card_at, t, area, scale, window, cx);
                self.card = current;
                let to = self.scene(card, t - card_at, t, area, scale, window, cx);
                DistortionWave::new(self.wave.clone(), from)
                    .revealed_content(to)
                    .size_full()
                    .into_any_element()
            }
        } else {
            self.release_wave(cx);
            if position.scene == Scene::Card
                && let Some(card) = &self.card
            {
                card.update(cx, |c, cx| c.set_snapshot(false, cx));
            }
            let mut fade = 1.;
            // In from black after a dip, and after a Skip.
            if let Some(previous) = position.index.checked_sub(1)
                && let Some(Transition::Dip { seconds }) = self.sheet.cues[previous].out
            {
                fade = beats::smooth(position.local / seconds);
            }
            if let Some(at) = self.skipped_at
                && t - at < SKIP_FADE
            {
                fade = beats::smooth((t - at) / SKIP_FADE);
            }
            if let Some((Transition::Dip { seconds }, p)) = position.leaving {
                let _ = seconds;
                fade = 1. - beats::smooth(p);
            }
            let scene = self.scene(position.index, position.local, t, area, scale, window, cx);
            div()
                .size_full()
                .bg(rgb(0x000000))
                .child(div().size_full().opacity(fade as f32).child(scene))
                .into_any_element()
        };
        // VoiceOver hears each scene's title as it starts.
        let title = self.sheet.cues[position.index].title;
        if self.spoken != Some(position.index) {
            self.spoken = Some(position.index);
        }
        let skippable = position.scene != Scene::Card && self.hand_off.is_none();
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
