//! The first-run stage: one borderless window over the whole display that
//! dims the desktop and runs the film on the cue sheet (`cue`). The mark
//! arrives out of light and folds into the agent's composer, which stays on
//! stage for the whole film: the real editor arrives around it, and each
//! request typed into it is applied to the editor through the agent's own
//! path (`Demo`), the wave carrying the change from the editor as it was
//! into the editor as it is now. The film ends on the Welcome card, where
//! the first-run steps play out. The room's light is the engine's
//! (`Pictures`), painted as a GPUI texture. AppKit buttons on the card
//! paint their own snapshot while a wave carries them.
pub mod beats;
pub mod cue;
mod freeze;
mod furniture;

use crate::native_control::{BuildSnapshot, ControlSpec, Kind, NativeControl};
use crate::wave::{DistortionWave, Palette, ReducedMotion, WaveRect, WaveState, WaveStyle};
use crate::welcome_view::WelcomeView;
pub use beats::Composing;
pub use cue::{Change, CueSheet, Scene, Transition};
pub use freeze::{Frozen, freeze};
use gpui::{prelude::*, *};
use pm_design::Tokens;
use std::{rc::Rc, sync::Arc, time::Instant};

/// A picture the engine draws for the stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Picture {
    /// The room: its light, grain and falloff, the whole display.
    Backdrop,
    /// The film's project, as the editor's viewer shows it.
    Viewer,
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
/// What the composer shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Composer {
    pub text: String,
    pub composing: Composing,
}
/// The editor the film changes, and the agent's composer: the app's real
/// ones (`demo_editor`), on the film's own project and kernel.
pub trait Demo {
    /// The live editor, laid out in `size` (its window on the stage).
    fn editor(&mut self, size: Size<Pixels>, window: &mut Window, cx: &mut App) -> AnyElement;
    /// The agent's composer showing `composer`.
    fn composer(&mut self, composer: &Composer, window: &mut Window, cx: &mut App) -> AnyElement;
    /// The composer's size on the stage.
    fn composer_size(&self) -> Size<Pixels>;
    /// Applies `change` through the agent's own path.
    fn apply(&mut self, change: Change, cx: &mut App);
    /// Whether `change` has reached the editor on screen.
    fn landed(&self, change: Change, cx: &App) -> bool;
    /// Where the changed surface sits, for the wave's source.
    fn site(&self, change: Change, area: Size<Pixels>) -> Bounds<Pixels>;
    /// Recordings: waits until everything asked for has landed.
    fn settle(&mut self, _cx: &mut App) {}
    /// The film's clock, each frame (the editor's own playhead follows it).
    fn at(&mut self, _t: f64) {}
}
/// A sound cue (`cue::Sound::id`) reached.
pub type OnSound = Rc<dyn Fn(&str, &mut App)>;

/// Seconds the room takes to fade in after a Skip, and out after the hand-off.
const SKIP_FADE: f64 = 0.35;
const LIGHTS_UP: f64 = 0.45;
/// Room under the editor's window for the composer.
const COMPOSER_ROOM: Pixels = px(150.);
/// A change still on its way this long after it was applied holds the
/// film (its clock stops on "Working…") until it lands, for at most
/// `MAX_HOLD` seconds; then the wave goes anyway.
const LAND_PATIENCE: f64 = 0.4;
const MAX_HOLD: f64 = 8.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WaveKey {
    Cue(usize),
    Change(Change),
    Step(u64),
    HandOff,
}
struct HandOff {
    start: f64,
    /// The app's first window as drawn, where it sits on the display.
    picture: Option<(Arc<RenderImage>, Bounds<Pixels>)>,
    done: Option<Box<dyn FnOnce(&mut Window, &mut App)>>,
}
/// How a beat's scene shows the editor.
enum EditorPicture {
    Live,
    /// Live, and pictured into the `Frozen` this frame.
    Capture(Frozen),
    /// The editor as it was, held while a change lands.
    Held(Option<Arc<RenderImage>>),
}
/// A change on its way: frozen, applied, landing.
struct Landing {
    change: Change,
    frozen: Frozen,
    applied: Option<f64>,
    landed: Option<f64>,
}

pub struct Stage {
    pub sheet: CueSheet,
    pub tokens: Tokens,
    pictures: Box<dyn Pictures>,
    demo: Option<Box<dyn Demo>>,
    opened: Instant,
    /// Tests and recordings pin the film's clock (seconds).
    pub clock: Option<f64>,
    /// Added to the clock: a Skip moves the film to the card.
    offset: f64,
    skipped_at: Option<f64>,
    card: Option<Entity<WelcomeView>>,
    /// The card as it was before a step changed it, for the step's wave.
    leaving_card: Option<(Entity<WelcomeView>, f64)>,
    steps: u64,
    wave: Entity<WaveState>,
    wave_for: Option<WaveKey>,
    landing: Option<Landing>,
    /// The film holds its clock while a change is still on its way:
    /// since when (raw clock), and how long it has held before.
    hold_from: Option<f64>,
    held: f64,
    last_t: f64,
    on_sound: Option<OnSound>,
    focus: FocusHandle,
    skip: Option<Entity<NativeControl>>,
    on_skip: Option<Rc<dyn Fn(&mut App)>>,
    hand_off: Option<HandOff>,
}

impl Stage {
    /// `demo` None (or `film` false): the card alone, no film.
    pub fn new(
        tokens: Tokens,
        sheet: CueSheet,
        pictures: Box<dyn Pictures>,
        demo: Option<Box<dyn Demo>>,
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
        let offset = if demo.is_some() { 0. } else { sheet.length() };
        Self {
            sheet,
            tokens,
            pictures,
            demo,
            opened: Instant::now(),
            clock: None,
            offset,
            skipped_at: None,
            card: None,
            leaving_card: None,
            steps: 0,
            wave,
            wave_for: None,
            landing: None,
            hold_from: None,
            held: 0.,
            last_t: -1.,
            on_sound: None,
            focus: cx.focus_handle(),
            skip: None,
            on_skip: None,
            hand_off: None,
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
        let raw = self.raw();
        // A hold stops the film where it began.
        raw.min(self.hold_from.unwrap_or(raw)) + self.offset - self.held
    }
    fn raw(&self) -> f64 {
        self.clock
            .unwrap_or_else(|| self.opened.elapsed().as_secs_f64())
    }
    pub fn card(&self) -> Option<&Entity<WelcomeView>> {
        self.card.as_ref()
    }
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }
    /// Recordings: lets the film's editor finish what it was asked.
    pub fn settle(&mut self, cx: &mut App) {
        if let Some(demo) = self.demo.as_mut() {
            demo.settle(cx);
        }
    }
    /// The Welcome card the film ends on; a later card (the next step)
    /// arrives with a wave from the one before.
    pub fn set_card(&mut self, card: Entity<WelcomeView>, cx: &mut Context<Self>) {
        let t = self.now();
        if let Some(previous) = self.card.replace(card)
            && self.sheet.at(t).scene == Scene::Card
        {
            self.steps += 1;
            self.leaving_card = Some((previous, t));
        }
        cx.notify();
    }
    /// Esc or Skip: straight to the card, with a short fade from black.
    pub fn skip(&mut self, cx: &mut Context<Self>) {
        let t = self.now();
        let card = self.sheet.length();
        if t >= card {
            return;
        }
        if let Some(from) = self.hold_from.take() {
            self.held += self.raw() - from;
        }
        let t = self.now();
        self.offset += card - t;
        self.skipped_at = Some(card);
        self.landing = None;
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
        (0.94 * beats::span(t, 0., self.sheet.cues[1].at)) as f32
    }
    /// The room fades up over the veil once the house lights are down.
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
    /// The composer alone on the stage: centred, a little low.
    fn composer_alone(area: Size<Pixels>, home: Bounds<Pixels>) -> Bounds<Pixels> {
        Bounds::new(
            point(
                (area.width - home.size.width) / 2.,
                (area.height - home.size.height) / 2. + area.height * 0.06,
            ),
            home.size,
        )
    }
    /// The editor's window on the stage: the app as it will open, standing
    /// in the room rather than filling the display.
    pub fn editor_rect(area: Size<Pixels>) -> Bounds<Pixels> {
        let margin = area.height * 0.03;
        let size = size(area.width * 0.9, area.height - margin * 2. - COMPOSER_ROOM);
        Bounds::new(point((area.width - size.width) / 2., margin), size)
    }
    /// The composer's place in the film: on the stage, under the app's
    /// window, so it never covers what it changes.
    fn composer_home(area: Size<Pixels>, composer: Size<Pixels>) -> Bounds<Pixels> {
        let rect = Self::editor_rect(area);
        Bounds::new(
            point(
                (area.width - composer.width) / 2.,
                rect.origin.y + rect.size.height + (COMPOSER_ROOM - composer.height) / 2.,
            ),
            composer,
        )
    }
    fn room_picture(&mut self, t: f64, area: Size<Pixels>, scale: f32) -> AnyElement {
        // The grain moves at film's 24 frames a second.
        let room_t = (t.min(self.sheet.length() + 1.5) * 24.).floor() / 24.;
        let size = (
            ((f32::from(area.width) * scale * 0.5).round() as u32).max(1),
            ((f32::from(area.height) * scale * 0.5).round() as u32).max(1),
        );
        texture_fill(self.pictures.picture(Picture::Backdrop, room_t, size)).into_any_element()
    }
    /// One scene as a picture at `local` seconds into it (the composer is
    /// drawn over it separately: it stays live through every wave).
    fn scene(
        &mut self,
        index: usize,
        local: f64,
        t: f64,
        area: Size<Pixels>,
        scale: f32,
        editor: EditorPicture,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scene = self.sheet.cues[index].scene;
        let motion = !self.reduce_motion();
        let mut root = div()
            .id(("stage-scene", index))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x000000))
            .child(self.room_picture(t, area, scale));
        root = match scene {
            Scene::HouseLights | Scene::Composer => root,
            Scene::Mark => {
                let fold = match self.sheet.at(t).leaving {
                    Some((Transition::Fold { .. }, p)) if self.sheet.at(t).index == index => p,
                    _ => 0.,
                };
                root.child(furniture::mark(local, fold, &self.tokens, motion))
            }
            Scene::Beat(_) => {
                let rect = Self::editor_rect(area);
                let radius = px(self.tokens.radius.window);
                let content = match (editor, self.demo.as_mut()) {
                    (EditorPicture::Held(image), _) => still(image),
                    (EditorPicture::Live, Some(demo)) => demo.editor(rect.size, window, cx),
                    (EditorPicture::Capture(frozen), Some(demo)) => {
                        freeze(demo.editor(rect.size, window, cx), frozen, true).into_any_element()
                    }
                    (_, None) => div().into_any_element(),
                };
                root.child(
                    div()
                        .absolute()
                        .left(rect.origin.x)
                        .top(rect.origin.y)
                        .w(rect.size.width)
                        .h(rect.size.height)
                        .rounded(radius)
                        .shadow_2xl()
                        .border_1()
                        .border_color(rgba((self.tokens.colors.text << 8) | 0x1a))
                        .overflow_hidden()
                        .child(content),
                )
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
        root.into_any_element()
    }
    /// The composer, where it stands at `t`: grown out of the folded mark,
    /// centred alone, then at home in the editor; gone on the card.
    fn composer(
        &mut self,
        t: f64,
        area: Size<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let position = self.sheet.at(t);
        let reduce_motion = self.reduce_motion();
        let demo = self.demo.as_mut()?;
        let home = Self::composer_home(area, demo.composer_size());
        let alone = Self::composer_alone(area, home);
        let (bounds, opacity, composer) = match position.scene {
            Scene::Mark => {
                let Some((Transition::Fold { .. }, p)) = position.leaving else {
                    return None;
                };
                let grow = beats::span(p, 0.35, 1.);
                let width = alone.size.width * (0.12 + 0.88 * grow as f32);
                let b = Bounds::new(
                    point((area.width - width) / 2., alone.origin.y),
                    size(width, alone.size.height),
                );
                (
                    b,
                    grow as f32,
                    Composer {
                        text: String::new(),
                        composing: Composing::Typing(0),
                    },
                )
            }
            Scene::Composer => {
                // The composer travels home under the wave into the editor;
                // under Reduce Motion it fades out here and in at home.
                let p = match position.leaving {
                    Some((Transition::Wave(_), p)) => p,
                    _ => 0.,
                };
                let (travel, opacity) = if reduce_motion {
                    let home_now = p >= 0.5;
                    (
                        if home_now { 1. } else { 0. },
                        (1. - 2. * p).abs().max(0.) as f32,
                    )
                } else {
                    (beats::smooth(p) as f32, 1.)
                };
                let b = Bounds::new(
                    alone.origin + (home.origin - alone.origin) * travel,
                    alone.size,
                );
                (
                    b,
                    opacity,
                    Composer {
                        text: String::new(),
                        composing: Composing::Typing(0),
                    },
                )
            }
            Scene::Beat(change) => {
                let fade = match position.leaving {
                    Some((Transition::Wave(_), p)) => 1. - beats::span(p, 0., 0.5) as f32,
                    _ => 1.,
                };
                let composing = beats::composing(change, position.local);
                let text = match composing {
                    Composing::Typing(n) => change.prompt().chars().take(n).collect(),
                    _ => change.prompt().to_owned(),
                };
                (home, fade, Composer { text, composing })
            }
            _ => return None,
        };
        if opacity <= 0. {
            return None;
        }
        Some(
            div()
                .absolute()
                .left(bounds.origin.x)
                .top(bounds.origin.y)
                .w(bounds.size.width)
                .h(bounds.size.height)
                .opacity(opacity)
                .child(demo.composer(&composer, window, cx))
                .into_any_element(),
        )
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
                            Corners::default(),
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
    /// A beat's change: hold the editor as it was, apply the change, and
    /// once it has landed wave from the held picture into the live editor.
    fn beat(
        &mut self,
        change: Change,
        index: usize,
        local: f64,
        t: f64,
        area: Size<Pixels>,
        scale: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if local < beats::LANDS {
            self.landing = None;
            return self.scene(
                index,
                local,
                t,
                area,
                scale,
                EditorPicture::Live,
                window,
                cx,
            );
        }
        if self.landing.as_ref().is_none_or(|l| l.change != change) {
            // This frame pictures the editor as it was; the change goes in
            // after the frame.
            let frozen = Frozen::default();
            self.landing = Some(Landing {
                change,
                frozen: frozen.clone(),
                applied: None,
                landed: None,
            });
            return self.scene(
                index,
                local,
                t,
                area,
                scale,
                EditorPicture::Capture(frozen),
                window,
                cx,
            );
        }
        let landing = self.landing.as_mut().unwrap();
        if landing.applied.is_none() {
            landing.applied = Some(t);
            if let Some(demo) = self.demo.as_mut() {
                demo.apply(change, cx);
            }
        }
        let waiting = self
            .landing
            .as_ref()
            .and_then(|l| l.landed.is_none().then_some(l.applied));
        if let Some(Some(applied)) = waiting {
            let arrived = self.demo.as_ref().is_some_and(|d| d.landed(change, cx));
            let raw = self.raw();
            let holding = self.hold_from.map_or(0., |from| raw - from);
            if arrived || holding > MAX_HOLD {
                // The film goes on from where it held.
                if let Some(from) = self.hold_from.take() {
                    self.held += raw - from;
                }
                let now = self.now();
                if let Some(landing) = self.landing.as_mut() {
                    landing.landed = Some(now);
                }
            } else if t - applied > LAND_PATIENCE && self.hold_from.is_none() {
                // Still working: the film waits with it, on "Working…".
                self.hold_from = Some(raw);
            }
        }
        let landing = self.landing.as_ref().unwrap();
        let held = landing.frozen.get();
        let Some(landed) = landing.landed else {
            // Not there yet: the editor as it was, held.
            return self.scene(
                index,
                local,
                t,
                area,
                scale,
                EditorPicture::Held(held),
                window,
                cx,
            );
        };
        let p = (t - landed) / self.sheet.wave_seconds;
        if p >= 1. || held.is_none() {
            self.release_wave(cx);
            return self.scene(
                index,
                local,
                t,
                area,
                scale,
                EditorPicture::Live,
                window,
                cx,
            );
        }
        let source = self
            .demo
            .as_ref()
            .map_or(Bounds::new(point(px(0.), px(0.)), area), |d| {
                let rect = Self::editor_rect(area);
                let site = d.site(change, rect.size);
                Bounds::new(rect.origin + site.origin, site.size)
            });
        self.arm_wave(WaveKey::Change(change), cue::STEP_WAVE, source, window, cx);
        let from = self.scene(
            index,
            local,
            t,
            area,
            scale,
            EditorPicture::Held(held),
            window,
            cx,
        );
        let to = self.scene(
            index,
            local,
            t,
            area,
            scale,
            EditorPicture::Live,
            window,
            cx,
        );
        self.waved(p, from, to, cx)
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
/// A held picture filling its parent.
fn still(image: Option<Arc<RenderImage>>) -> AnyElement {
    div()
        .relative()
        .size_full()
        .bg(rgb(0x000000))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    if let Some(image) = image {
                        let _ = window.paint_image(
                            bounds,
                            bounds,
                            Corners::default(),
                            0.,
                            image,
                            0,
                            false,
                        );
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .into_any_element()
}

impl Render for Stage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.now();
        self.sounds(t, cx);
        if let Some(demo) = self.demo.as_mut() {
            demo.at(t);
        }
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
        // The house lights go down from when the stage opens, film or not.
        let opened = self.raw();
        let mut stage_alpha = self.room(opened);
        let mut veil = self.veil(opened);
        let card_index = self.sheet.cues.len() - 1;
        let picture: AnyElement = if self.hand_off.is_some() {
            let start = self.hand_off.as_ref().unwrap().start;
            let p = ((t - start) / self.sheet.wave_seconds).clamp(0., 1.);
            let up = ((t - start - self.sheet.wave_seconds) / LIGHTS_UP).clamp(0., 1.);
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
                    .unwrap_or_else(|| Bounds::new(point(px(0.), px(0.)), area));
                self.arm_wave(WaveKey::HandOff, cue::STEP_WAVE, source, window, cx);
                if let Some(card) = &self.card {
                    card.update(cx, |c, cx| c.set_snapshot(true, cx));
                }
                let from = self.scene(
                    card_index,
                    t - card_at,
                    t,
                    area,
                    scale,
                    EditorPicture::Live,
                    window,
                    cx,
                );
                let to = self.arrival(self.hand_off.as_ref().unwrap());
                self.waved(p, from, to, cx)
            } else {
                self.release_wave(cx);
                self.arrival(self.hand_off.as_ref().unwrap())
            }
        } else if let Some((Transition::Wave(kind), p)) = position.leaving
            && p > 0.
        {
            let next = position.index + 1;
            let source = match self.sheet.cues[position.index].scene {
                Scene::Composer => {
                    let home = self
                        .demo
                        .as_ref()
                        .map(|d| Self::composer_home(area, d.composer_size()));
                    home.map_or(Bounds::new(point(px(0.), px(0.)), area), |h| {
                        Self::composer_alone(area, h)
                    })
                }
                _ => Bounds::new(point(px(0.), px(0.)), area),
            };
            self.arm_wave(WaveKey::Cue(position.index), kind, source, window, cx);
            if self.sheet.cues[next].scene == Scene::Card
                && let Some(card) = &self.card
            {
                card.update(cx, |c, cx| c.set_snapshot(true, cx));
            }
            let started = position.length - self.sheet.wave_seconds;
            let from = self.scene(
                position.index,
                started,
                t,
                area,
                scale,
                EditorPicture::Live,
                window,
                cx,
            );
            let to = self.scene(next, 0., t, area, scale, EditorPicture::Live, window, cx);
            self.waved(p, from, to, cx)
        } else if let Some((previous, began)) = self.leaving_card.clone() {
            // A step on the card: the wave from the card before.
            let p = (t - began) / self.sheet.wave_seconds;
            if p >= 1. {
                self.leaving_card = None;
                self.release_wave(cx);
                if let Some(card) = &self.card {
                    card.update(cx, |c, cx| c.set_snapshot(false, cx));
                }
                self.scene(
                    card_index,
                    t - card_at,
                    t,
                    area,
                    scale,
                    EditorPicture::Live,
                    window,
                    cx,
                )
            } else {
                let key = WaveKey::Step(self.steps);
                let card_bounds = Bounds::new(point(px(0.), px(0.)), area);
                self.arm_wave(key, cue::STEP_WAVE, card_bounds, window, cx);
                previous.update(cx, |c, cx| c.set_snapshot(true, cx));
                if let Some(card) = &self.card {
                    card.update(cx, |c, cx| c.set_snapshot(true, cx));
                }
                let current = self.card.replace(previous);
                let from = self.scene(
                    card_index,
                    t - card_at,
                    t,
                    area,
                    scale,
                    EditorPicture::Live,
                    window,
                    cx,
                );
                self.card = current;
                let to = self.scene(
                    card_index,
                    t - card_at,
                    t,
                    area,
                    scale,
                    EditorPicture::Live,
                    window,
                    cx,
                );
                self.waved(p, from, to, cx)
            }
        } else if let Scene::Beat(change) = position.scene {
            self.beat(
                change,
                position.index,
                position.local,
                t,
                area,
                scale,
                window,
                cx,
            )
        } else {
            self.release_wave(cx);
            if position.scene == Scene::Card
                && let Some(card) = &self.card
            {
                card.update(cx, |c, cx| c.set_snapshot(false, cx));
            }
            let mut fade = 1.;
            if let Some(at) = self.skipped_at
                && t - at < SKIP_FADE
            {
                fade = beats::smooth((t - at) / SKIP_FADE);
            }
            let scene = self.scene(
                position.index,
                position.local,
                t,
                area,
                scale,
                EditorPicture::Live,
                window,
                cx,
            );
            div()
                .size_full()
                .bg(rgb(0x000000))
                .child(div().size_full().opacity(fade as f32).child(scene))
                .into_any_element()
        };
        let composer = if self.hand_off.is_none() {
            self.composer(t, area, window, cx)
        } else {
            None
        };
        let title = self.sheet.cues[position.index].title;
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
            .children(composer.map(|c| div().absolute().inset_0().opacity(stage_alpha).child(c)))
            .child(
                // VoiceOver hears each scene's title (each request) as it starts.
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
