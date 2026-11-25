//! Reusable snapshot transition (the first-run stage's wave).
//! Own the handle in a view; build a `DistortionWave` each render. Give both
//! children opaque backgrounds. Platform child windows and deferred popovers
//! cannot be pictured; put live controls/video in `overlay` instead.
use gpui::{prelude::*, *};
pub use pm_wave::{
    Palette, Point as WavePoint, Rect as WaveRect, ValidationError, WaveStyle, WaveType,
};
use pm_wave::{WaveClock, WaveFrame, WaveRenderer};
use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

/// An element's source rectangle, measured by `track`, or a local rectangle.
#[derive(Clone)]
pub struct WaveSource(Rc<Cell<Option<Bounds<Pixels>>>>);
impl Default for WaveSource {
    fn default() -> Self {
        Self(Rc::new(Cell::new(None)))
    }
}
impl WaveSource {
    /// Wrap the source control in this measuring element (usually in the overlay).
    pub fn track(&self, child: impl IntoElement) -> impl IntoElement {
        let measured = self.0.clone();
        div().relative().child(child).child(
            canvas(move |b, _, _| measured.set(Some(b)), |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
    }
    fn local(&self, bounds: Bounds<Pixels>) -> Option<WaveRect> {
        self.0.get().map(|s| WaveRect {
            x: f64::from(s.origin.x - bounds.origin.x),
            y: f64::from(s.origin.y - bounds.origin.y),
            w: f64::from(s.size.width),
            h: f64::from(s.size.height),
        })
    }
}
#[derive(Clone)]
pub enum Source {
    Rect(WaveRect),
    Element(WaveSource),
}
impl From<WaveRect> for Source {
    fn from(v: WaveRect) -> Self {
        Self::Rect(v)
    }
}
impl From<WaveSource> for Source {
    fn from(v: WaveSource) -> Self {
        Self::Element(v)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReducedMotion {
    #[default]
    System,
    Always,
    Never,
}
type Finish = Box<dyn FnMut(bool, &mut App)>;
/// State/control handle for `DistortionWave`. Its constructor validates the style.
pub struct WaveState {
    clock: WaveClock,
    style: WaveStyle,
    source: Source,
    focus: Option<WavePoint>,
    corner_radius: f64,
    reduced_motion: ReducedMotion,
    renderer: Option<WaveRenderer>,
    last_frame: Option<Arc<NativeTexture>>,
    prepare: bool,
    last_tick: Option<Instant>,
    queued: bool,
    on_finish: Option<Finish>,
    last_error: Option<String>,
    prepared_size: Option<Size<Pixels>>,
    /// Preparation failed: the run cuts at its midpoint instead of
    /// capturing again every frame (until `clear`/`reset`).
    failed: bool,
}
impl WaveState {
    pub fn new(style: WaveStyle, source: impl Into<Source>) -> Result<Self, ValidationError> {
        style.validate()?;
        let source = source.into();
        if let Source::Rect(r) = &source {
            r.validate()?;
        }
        Ok(Self {
            corner_radius: style.corner_radius,
            clock: WaveClock::default(),
            style,
            source,
            focus: None,
            reduced_motion: ReducedMotion::System,
            renderer: None,
            last_frame: None,
            prepare: false,
            last_tick: None,
            queued: false,
            on_finish: None,
            last_error: None,
            prepared_size: None,
            failed: false,
        })
    }
    pub fn progress(&self) -> f64 {
        self.clock.progress()
    }
    pub fn error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
    pub fn is_running(&self) -> bool {
        self.clock.is_running()
    }
    pub fn on_finish(&mut self, callback: impl FnMut(bool, &mut App) + 'static) {
        self.on_finish = Some(Box::new(callback));
    }
    pub fn set_reduced_motion(&mut self, motion: ReducedMotion, window: &mut Window) {
        self.reduced_motion = motion;
        window.refresh();
    }
    pub fn set_style(
        &mut self,
        style: WaveStyle,
        window: &mut Window,
    ) -> Result<(), ValidationError> {
        style.validate()?;
        self.style = style;
        self.invalidate();
        window.refresh();
        Ok(())
    }
    pub fn set_corner_radius(
        &mut self,
        radius: f64,
        window: &mut Window,
    ) -> Result<(), ValidationError> {
        if !radius.is_finite() || radius < 0. {
            return Err(ValidationError(
                "cornerRadius must be finite and nonnegative",
            ));
        }
        self.corner_radius = radius;
        self.invalidate();
        window.refresh();
        Ok(())
    }
    /// Local click point; `None` restores the source centre. Source clicks are tracked automatically.
    pub fn set_focus(&mut self, focus: Option<WavePoint>) -> Result<(), ValidationError> {
        if focus.is_some_and(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return Err(ValidationError("focus must be finite"));
        }
        self.focus = focus;
        Ok(())
    }
    pub fn set_source(&mut self, source: impl Into<Source>) -> Result<(), ValidationError> {
        let source = source.into();
        if let Source::Rect(r) = &source {
            r.validate()?;
        }
        self.source = source;
        Ok(())
    }
    pub fn set_revealed(&mut self, revealed: bool, window: &mut Window) {
        self.clock.set_revealed(revealed);
        self.last_tick = Some(Instant::now());
        window.refresh();
    }
    pub fn play(&mut self, window: &mut Window) {
        self.clock.play();
        self.last_tick = Some(Instant::now());
        window.refresh();
    }
    pub fn reverse(&mut self, window: &mut Window) {
        self.set_revealed(false, window);
    }
    pub fn reset(&mut self, window: &mut Window) {
        self.clear();
        window.refresh();
    }
    pub fn scrub(&mut self, progress: f64, window: &mut Window) -> Result<(), ValidationError> {
        self.seek(progress)?;
        window.refresh();
        Ok(())
    }
    /// `scrub` from inside a render: the caller is drawing this frame.
    pub fn seek(&mut self, progress: f64) -> Result<(), ValidationError> {
        self.clock.scrub(progress)?;
        self.last_tick = None;
        if self.progress() <= 0. || self.progress() >= 1. {
            self.release();
        }
        Ok(())
    }
    /// `reset` from inside a render.
    pub fn clear(&mut self) {
        self.failed = false;
        self.clock.reset();
        self.release();
        self.prepare = false;
    }
    /// `prepare` from inside a render: both children are pictured on the
    /// next paint, before the run that needs them.
    pub fn prepare_now(&mut self) {
        self.release();
        self.prepare = true;
    }
    /// Captures both children on the next paint, even while idle.
    pub fn prepare(&mut self, window: &mut Window) {
        self.release();
        self.prepare = true;
        window.refresh();
    }
    fn invalidate(&mut self) {
        self.release();
    }
    fn release(&mut self) {
        self.last_frame = None;
        if let Some(r) = self.renderer.as_mut() {
            r.release();
        }
        self.prepared_size = None;
        self.last_tick = None;
    }
    fn motionless(&self) -> bool {
        match self.reduced_motion {
            ReducedMotion::Always => true,
            ReducedMotion::Never => false,
            ReducedMotion::System => crate::native::settings().reduce_motion,
        }
    }
    fn source_rect(&self, bounds: Bounds<Pixels>) -> Option<WaveRect> {
        match &self.source {
            Source::Rect(r) => Some(*r),
            Source::Element(s) => s.local(bounds),
        }
    }
    fn tick(&mut self, cx: &mut App) {
        let now = Instant::now();
        if self.clock.is_running()
            && let Some(last) = self.last_tick.replace(now)
        {
            self.clock.tick(
                now.duration_since(last).as_secs_f64(),
                &self.style,
                self.motionless(),
            );
        }
        if let Some(done) = self.clock.take_finish() {
            self.release();
            if let Some(finish) = self.on_finish.as_mut() {
                finish(done, cx);
            }
        }
    }
}

/// Two stacked views, frozen only during the wave; the overlay stays live above it.
/// The root takes the larger child's size and supports the usual `Styled` sizing.
pub struct DistortionWave {
    state: Entity<WaveState>,
    from: AnyElement,
    to: Option<AnyElement>,
    overlay: Option<AnyElement>,
    style: StyleRefinement,
}
impl DistortionWave {
    pub fn new(state: Entity<WaveState>, from: impl IntoElement) -> Self {
        Self {
            state,
            from: div()
                .id("from")
                .col_start(1)
                .row_start(1)
                .child(from)
                .into_any_element(),
            to: None,
            overlay: None,
            style: StyleRefinement::default(),
        }
    }
    pub fn revealed_content(mut self, to: impl IntoElement) -> Self {
        self.to = Some(
            div()
                .id("to")
                .col_start(1)
                .row_start(1)
                .child(to)
                .into_any_element(),
        );
        self
    }
    pub fn overlay(mut self, overlay: impl IntoElement) -> Self {
        self.overlay = Some(
            div()
                .id("overlay")
                .col_start(1)
                .row_start(1)
                .child(overlay)
                .into_any_element(),
        );
        self
    }
}
impl Styled for DistortionWave {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
impl IntoElement for DistortionWave {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
#[derive(Debug)]
struct FrameLease {
    _frame: WaveFrame,
}
impl TextureLease for FrameLease {
    fn gpu_completed(&self, _: bool) {}
}
impl Element for DistortionWave {
    type RequestLayoutState = ();
    type PrepaintState = (bool, bool);
    fn id(&self) -> Option<ElementId> {
        Some(ElementId::View(self.state.entity_id()))
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style {
            display: Display::Grid,
            ..Style::default()
        };
        style.refine(&self.style);
        let radii = &self.style.corner_radii;
        if [
            radii.top_left,
            radii.top_right,
            radii.bottom_left,
            radii.bottom_right,
        ]
        .iter()
        .any(Option::is_some)
        {
            let r = style.corner_radii.to_pixels(window.rem_size());
            let radius = f64::from(
                r.top_left
                    .max(r.top_right)
                    .max(r.bottom_left)
                    .max(r.bottom_right),
            );
            self.state.update(cx, |s, _| {
                if s.corner_radius != radius {
                    s.corner_radius = radius;
                    s.invalidate();
                }
            });
        }

        let mut children = vec![self.from.request_layout(window, cx)];
        if let Some(to) = self.to.as_mut() {
            children.push(to.request_layout(window, cx));
        }
        if let Some(overlay) = self.overlay.as_mut() {
            children.push(overlay.request_layout(window, cx));
        }
        (window.request_layout(style, children, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> (bool, bool) {
        let (capture, active, revealed) = self.state.update(cx, |s, cx| {
            s.tick(cx);
            if s.prepared_size.is_some_and(|size| size != bounds.size) {
                s.invalidate();
            }
            let p = s.progress();
            if s.failed {
                // No wave: a cut at the midpoint.
                return (false, false, p >= 0.5);
            }
            let active = p > 0. && p < 1.;
            let capture = (s.prepare || active)
                && !s.renderer.as_ref().is_some_and(WaveRenderer::is_prepared);
            (capture, active, p >= 1.)
        });
        if capture || !active && (!revealed || self.to.is_none()) {
            self.from.prepaint(window, cx);
        }
        if let Some(to) = self.to.as_mut()
            && (capture || !active && revealed)
        {
            to.prepaint(window, cx);
        }
        if active {
            window.insert_hitbox(bounds, HitboxBehavior::BlockMouse);
        }
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.prepaint(window, cx);
        }
        (capture, active)
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut (bool, bool),
        window: &mut Window,
        cx: &mut App,
    ) {
        let handle = self.state.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
            if phase == DispatchPhase::Capture && bounds.contains(&event.position) {
                handle.update(cx, |s, _| {
                    let point = WavePoint {
                        x: f64::from(event.position.x - bounds.origin.x),
                        y: f64::from(event.position.y - bounds.origin.y),
                    };
                    if s.source_rect(bounds).is_some_and(|r| {
                        point.x >= r.x
                            && point.y >= r.y
                            && point.x <= r.x + r.w
                            && point.y <= r.y + r.h
                    }) {
                        s.focus = Some(point);
                    }
                });
            }
        });
        let (capture, active) = *prepaint;
        if capture {
            // Pictured on the GPU: no readback, no wait, no upload; the
            // wave's preparation waits on the pictures' events on the GPU.
            let from = window.paint_snapshot_gpu(bounds, cx, |w, c| self.from.paint(w, c));
            let to = self
                .to
                .as_mut()
                .map(|to| window.paint_snapshot_gpu(bounds, cx, |w, c| to.paint(w, c)));
            self.state.update(cx, |s, _| {
                let result = (|| -> Result<(), String> {
                    if s.renderer.is_none() {
                        // The window's own device; a headless window (offscreen
                        // captures) draws on the system's.
                        let device = window
                            .gpu_context()
                            .and_then(|c| c.downcast::<metal::Device>().ok())
                            .map(|d| *d)
                            .or_else(metal::Device::system_default)
                            .ok_or("no Metal device for the wave")?;
                        s.renderer = Some(WaveRenderer::new(&device)?);
                    }
                    let from = from.map_err(|e| e.to_string())?;
                    let to = to.transpose().map_err(|e| e.to_string())?;
                    let width = f64::from(bounds.size.width);
                    let height = f64::from(bounds.size.height);
                    let k = WaveStyle::scale(width, height);
                    let style = s.style.for_size(width, height, s.corner_radius);
                    fn picture(p: &GpuSnapshot) -> pm_wave::GpuPicture<'_> {
                        pm_wave::GpuPicture {
                            texture: &p.texture,
                            event: &p.event,
                            value: p.value,
                        }
                    }
                    s.renderer.as_mut().unwrap().prepare_gpu(
                        picture(&from),
                        to.as_ref().map(picture),
                        width / k,
                        &style,
                    )?;
                    s.prepared_size = Some(bounds.size);
                    s.prepare = false;
                    Ok(())
                })();
                if let Err(error) = &result {
                    eprintln!("The wave could not prepare: {error}");
                    s.failed = true;
                }
                s.last_error = result.err();
            });
        }
        let texture = self.state.update(cx, |s, _| {
            if !active && !capture {
                return None;
            }
            let result = (|| -> Result<Arc<NativeTexture>, String> {
                let source = s
                    .source_rect(bounds)
                    .ok_or("source element has not been laid out")?;
                let width = f64::from(bounds.size.width);
                let height = f64::from(bounds.size.height);
                let k = WaveStyle::scale(width, height);
                let source = WaveRect {
                    x: source.x / k,
                    y: source.y / k,
                    w: source.w / k,
                    h: source.h / k,
                };
                let focus = s.focus.map(|p| WavePoint {
                    x: p.x / k,
                    y: p.y / k,
                });
                let reduced = s.motionless();
                let p = s.progress();
                let r = s.renderer.as_mut().ok_or("renderer is unavailable")?;
                let frame = r.render(p, source, width / k, height / k, focus, reduced)?;
                let texture = frame.texture().to_owned();
                let event = frame.event.clone();
                let value = frame.value;
                NativeTexture::new(
                    texture,
                    event,
                    value,
                    Arc::new(FrameLease { _frame: frame }),
                )
                .map_err(str::to_string)
            })();
            match result {
                Ok(t) => {
                    s.last_frame = Some(t.clone());
                    Some(t)
                }
                Err(e) => {
                    s.last_error = Some(e);
                    s.last_frame.clone()
                }
            }
        });
        if let Some(texture) = texture {
            window.paint_texture(bounds, texture);
        } else if !capture && !active {
            let radius = px(self.state.read(cx).corner_radius as f32);
            let mask = ContentMask {
                bounds,
                shape: bounds,
                corner_radii: Corners::all(radius),
                ..ContentMask::default()
            };
            window.with_content_mask(Some(mask), |window| {
                let state = self.state.read(cx);
                let revealed = state.progress() >= if state.failed { 0.5 } else { 1. };
                if revealed {
                    if let Some(to) = self.to.as_mut() {
                        to.paint(window, cx);
                    } else {
                        self.from.paint(window, cx);
                    }
                } else {
                    self.from.paint(window, cx);
                }
            });
        }
        if let Some(overlay) = self.overlay.as_mut() {
            let radius = px(self.state.read(cx).corner_radius as f32);
            let mask = ContentMask {
                bounds,
                shape: bounds,
                corner_radii: Corners::all(radius),
                ..ContentMask::default()
            };
            window.with_content_mask(Some(mask), |window| overlay.paint(window, cx));
        }
        self.state.update(cx, |s, _| {
            if s.is_running() && !s.queued {
                s.queued = true;
                let weak = self.state.downgrade();
                window.on_next_frame(move |window, cx| {
                    if let Some(state) = weak.upgrade() {
                        state.update(cx, |s, _| s.queued = false);
                        window.refresh();
                    }
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        state: Entity<WaveState>,
        overlay: u32,
    }
    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            DistortionWave::new(self.state.clone(), div().size_full().bg(rgb(0x285078)))
                .revealed_content(div().size_full().bg(rgb(0xc8a050)))
                .overlay(
                    div()
                        .absolute()
                        .top_2()
                        .left_2()
                        .w_3()
                        .h_3()
                        .bg(rgb(self.overlay)),
                )
                .w(px(64.))
                .h(px(48.))
        }
    }
    #[::core::prelude::v1::test]
    fn gpui_snapshot_overlay_and_finish_release_are_offscreen() {
        let device = metal::Device::system_default().unwrap();
        let renderer = WaveRenderer::new(&device).unwrap();
        let mut cx = HeadlessAppContext::with_platform(
            Arc::new(gpui_parley::ParleyTextSystem::new(
                gpui_parley::SystemFonts::Load,
            )),
            Arc::new(()),
            || {
                Some(Box::new(
                    gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                ))
            },
        );
        let window = cx
            .open_window(size(px(64.), px(48.)), |_, cx| {
                cx.new(|cx| {
                    let state = cx.new(|_| {
                        let mut state = WaveState::new(
                            WaveStyle {
                                corner_radius: 0.,
                                ..WaveStyle::default()
                            },
                            WaveRect {
                                x: 24.,
                                y: 36.,
                                w: 16.,
                                h: 8.,
                            },
                        )
                        .unwrap();
                        state.renderer = Some(renderer);
                        state.reduced_motion = ReducedMotion::Never;
                        state
                    });
                    Fixture {
                        state,
                        overlay: 0xff0000,
                    }
                })
            })
            .unwrap();
        let draw = |cx: &mut HeadlessAppContext| {
            cx.update_window(window.into(), |_, w, cx| {
                w.refresh();
                w.draw(cx).clear(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.capture_screenshot(window.into()).unwrap()
        };
        let before = draw(&mut cx);
        assert_eq!(before.get_pixel(60, 40).0, [40, 80, 120, 255]);
        cx.update_window(window.into(), |view, w, cx| {
            let view = view.downcast::<Fixture>().unwrap();
            view.update(cx, |v, cx| {
                v.state.update(cx, |s, _| s.scrub(0.3, w).unwrap())
            });
        })
        .unwrap();
        let during = draw(&mut cx);
        cx.update_window(window.into(), |view, w, cx| {
            let view = view.downcast::<Fixture>().unwrap();
            view.update(cx, |v, cx| {
                v.overlay = 0x00ff00;
                v.state.update(cx, |s, _| {
                    assert!(s.error().is_none(), "{:?}", s.error());
                    assert!(s.renderer.as_ref().unwrap().is_prepared());
                });
                w.refresh();
            });
        })
        .unwrap();
        let changed = draw(&mut cx);
        assert_eq!(during.get_pixel(20, 20).0, [255, 0, 0, 255]);
        assert_eq!(changed.get_pixel(20, 20).0, [0, 255, 0, 255]);
        cx.update_window(window.into(), |view, w, cx| {
            let view = view.downcast::<Fixture>().unwrap();
            view.update(cx, |v, cx| {
                v.state.update(cx, |s, _| {
                    s.clock.scrub(0.999).unwrap();
                    s.clock.set_revealed(true);
                    s.last_tick = Some(Instant::now() - std::time::Duration::from_secs(10));
                    w.refresh();
                })
            });
        })
        .unwrap();
        let after = draw(&mut cx);
        assert_eq!(after.get_pixel(60, 40).0, [200, 160, 80, 255]);
        // GPUI retains the previous scene until the next paint, even after GPU completion.
        draw(&mut cx);
        draw(&mut cx);
        cx.update_window(window.into(), |view, _, cx| {
            let view = view.downcast::<Fixture>().unwrap();
            view.update(cx, |v, cx| {
                v.state.update(cx, |s, _| {
                    assert!(!s.is_running());
                    assert_eq!(s.renderer.as_ref().unwrap().texture_count(), 0);
                })
            });
        })
        .unwrap();
    }
}
