//! The timeline playhead's CALayers, driven by the display link (S3: no GPUI
//! repaint while playing): pm-design's `playhead` (v1's accent hairline with
//! a `panel` halo so it reads over strips of any hue, under a pin resting on
//! the foot of the ruler, as the paused timeline draws it), plus a motion
//! trail whose width
//! follows the head's measured speed through a critically damped spring
//! (v1's playhead). The spring runs only while the head moves.
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_core_graphics::{CGColor, CGMutablePath};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CAGradientLayer, CAShapeLayer, CATransaction};
use std::time::Instant;

/// v1's trail: width per point/second of head speed, spring response (s),
/// widest trail (pt) and its opacity at the head.
const TRAIL_GAIN: f64 = 0.3;
const TRAIL_RESPONSE: f64 = 0.15;
const TRAIL_MAX: f64 = 150.;
const TRAIL_ALPHA: f64 = 0.22;

pub(crate) fn colour(rgb: u32, alpha: f64) -> impl std::ops::Deref<Target = CGColor> {
    let c = |s: u32| ((rgb >> s) & 255) as f64 / 255.;
    CGColor::new_srgb(c(16), c(8), c(0), alpha)
}

/// The trail's width (pt, signed by direction) as a critically damped spring
/// toward the head's measured speed. Closed form, so uneven ticks stay stable.
#[derive(Default)]
struct Trail {
    at: Option<(f64, Instant)>,
    width: f64,
    velocity: f64,
}
impl Trail {
    fn step(&mut self, x: f64, now: Instant) -> f64 {
        let Some((last, then)) = self.at.replace((x, now)) else {
            return 0.;
        };
        let dt = now.duration_since(then).as_secs_f64().min(0.25);
        if dt <= 0. {
            return self.width;
        }
        let target = (TRAIL_GAIN * (x - last) / dt).clamp(-TRAIL_MAX, TRAIL_MAX);
        let omega = 2. / TRAIL_RESPONSE;
        let decay = (-omega * dt).exp();
        let error = self.width - target;
        let b = self.velocity + omega * error;
        self.width = target + (error + b * dt) * decay;
        self.velocity = (self.velocity - omega * b * dt) * decay;
        if self.width.abs() < 0.5 && target.abs() < 0.5 {
            (self.width, self.velocity) = (0., 0.);
        }
        self.width
    }
    fn reset(&mut self) {
        *self = Self::default();
    }
}

pub struct Playhead {
    line: Retained<AnyObject>,
    head: Retained<CAShapeLayer>,
    trail: Retained<CAGradientLayer>,
    spring: Trail,
    /// The graph's values on the head: a dot and a readout per curve.
    marks: Vec<(Retained<AnyObject>, Retained<AnyObject>)>,
    shown: Vec<pm_panel_timeline::graph_render::Mark>,
}
impl Playhead {
    /// `line` is the shared playhead layer; the trail goes in under it.
    pub fn new(parent: &AnyObject, line: Retained<AnyObject>) -> Self {
        let head = CAShapeLayer::new();
        let trail = CAGradientLayer::new();
        unsafe {
            let _: () = objc2::msg_send![parent, insertSublayer: &*trail, below: &*line];
            let _: () = objc2::msg_send![&*line, addSublayer: &*head];
        }
        trail.setStartPoint(NSPoint::new(0., 0.5));
        trail.setEndPoint(NSPoint::new(1., 0.5));
        trail.setHidden(true);
        let path = pin_path(
            pm_design::Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default())
                .playhead()
                .head as f64,
        );
        head.setPath(Some(&*path));
        head.setLineWidth(1.);
        Self {
            line,
            head,
            trail,
            spring: Trail::default(),
            marks: vec![],
            shown: vec![],
        }
    }
    /// Colours follow the tokens (a theme or appearance change).
    pub fn style(&self, t: &pm_design::Tokens, scale: f64) {
        let p = t.playhead();
        let accent = colour(p.color, 1.);
        let panel = colour(p.halo.0, 1.);
        unsafe {
            let _: () = objc2::msg_send![&*self.line, setBackgroundColor: &*accent];
            let _: () = objc2::msg_send![&*self.line, setShadowColor: &*panel];
            let _: () = objc2::msg_send![&*self.line, setShadowOpacity: p.halo.1];
            let _: () = objc2::msg_send![&*self.line, setShadowRadius: t.metrics.border as f64];
            let _: () = objc2::msg_send![&*self.line, setShadowOffset: NSSize::new(0., 0.)];
        }
        self.head.setFillColor(Some(&accent));
        self.head.setStrokeColor(None);
        self.head.setContentsScale(scale);
        let clear = colour(p.color, 0.);
        let glow = colour(p.color, TRAIL_ALPHA);
        let colours: [&AnyObject; 2] = [cf_object(&clear), cf_object(&glow)];
        let array = NSArray::from_slice(&colours);
        unsafe { self.trail.setColors(Some(&array)) };
    }
    /// Place the line (`line` pt wide, `tall` pt from `bottom` up, at `x` in
    /// the parent) and the trail over the tracks below the ruler, right of
    /// `tracks_left`. The trail's speed is `travel`, the head's time in points
    /// (seconds × zoom), as v1: scrolling the view is not the head moving.
    #[allow(clippy::too_many_arguments)]
    pub fn place(
        &mut self,
        x: f64,
        travel: f64,
        tracks_left: f64,
        bottom: f64,
        line: f64,
        tall: f64,
        ruler: f64,
        moving: bool,
        t: &pm_design::Tokens,
    ) {
        // As the paused timeline: the pin's point rests `head_foot` above
        // the foot of the ruler and the line starts just inside the pin.
        let p = t.playhead();
        let (d, foot) = (p.head as f64, p.head_foot as f64);
        let above = (ruler - foot - 1.).clamp(0., tall);
        let line_tall = tall - above;
        unsafe {
            let _: () = objc2::msg_send![&*self.line, setFrame: NSRect::new(
                NSPoint::new(x - line / 2., bottom), NSSize::new(line, line_tall))];
        }
        self.head.setFrame(NSRect::new(
            NSPoint::new((line - d) / 2., line_tall - 1.),
            NSSize::new(d, d),
        ));
        let width = if moving && !t.accessibility.reduce_motion {
            self.spring.step(travel, Instant::now())
        } else {
            self.spring.reset();
            0.
        };
        let span = width.abs();
        if span < 0.5 {
            if !self.trail.isHidden() {
                // Fade out rather than vanish when the head stops.
                CATransaction::begin();
                CATransaction::setDisableActions(false);
                CATransaction::setAnimationDuration(t.motion.exit);
                self.trail.setOpacity(0.);
                CATransaction::commit();
                self.trail.setHidden(true);
            }
            return;
        }
        let (start, end) = if width > 0. { (0., 1.) } else { (1., 0.) };
        self.trail.setStartPoint(NSPoint::new(start, 0.5));
        self.trail.setEndPoint(NSPoint::new(end, 0.5));
        // The glow stays over the tracks, never across the layer names.
        let left = if width > 0. { x - span } else { x };
        let (left, span) = (left.max(tracks_left), span - (tracks_left - left).max(0.));
        if span < 0.5 {
            self.trail.setHidden(true);
            return;
        }
        self.trail.setFrame(NSRect::new(
            NSPoint::new(left, bottom),
            NSSize::new(span, (tall - ruler).max(0.)),
        ));
        self.trail.setOpacity(1.);
        self.trail.setHidden(false);
    }
}
impl Playhead {
    /// The Value Graph's readouts on the head (v1 drawGraphTime): a 6 pt dot
    /// on each curve in its ink and its value right of the head, in the
    /// smallest type with tabular digits. `marks` are in window points (y
    /// down); `height` is the parent's, whose layers count y up.
    pub fn marks(
        &mut self,
        marks: &[pm_panel_timeline::graph_render::Mark],
        height: f64,
        scale: f64,
        t: &pm_design::Tokens,
    ) {
        if marks == self.shown.as_slice() {
            return;
        }
        if marks.is_empty() {
            return self.clear_marks();
        }
        let size = t.typography.minimum as f64;
        let tall = (size * t.typography.dense_leading as f64).ceil();
        while self.marks.len() < marks.len() {
            let parent: *mut AnyObject = unsafe { objc2::msg_send![&*self.line, superlayer] };
            let Some(parent) = (unsafe { parent.as_ref() }) else {
                return;
            };
            let dot: Retained<AnyObject> =
                unsafe { Retained::from_raw(objc2::msg_send![objc2::class!(CALayer), new]) }.unwrap();
            let text: Retained<AnyObject> =
                unsafe { Retained::from_raw(objc2::msg_send![objc2::class!(CATextLayer), new]) }
                    .unwrap();
            let font = objc2_app_kit::NSFont::monospacedDigitSystemFontOfSize_weight(
                size,
                unsafe { objc2_app_kit::NSFontWeightMedium },
            );
            // CATextLayer.font is a CFTypeRef; NSFont is toll-free bridged to CTFont.
            let font_ref = Retained::as_ptr(&font) as *const std::ffi::c_void;
            unsafe {
                let _: () = objc2::msg_send![&*dot, setCornerRadius: 3f64];
                let _: () = objc2::msg_send![&*text, setFont: font_ref];
                let _: () = objc2::msg_send![&*text, setFontSize: size];
                let _: () = objc2::msg_send![&*text, setAlignmentMode: objc2_quartz_core::kCAAlignmentLeft];
                let _: () = objc2::msg_send![parent, addSublayer: &*dot];
                let _: () = objc2::msg_send![parent, addSublayer: &*text];
            }
            self.marks.push((dot, text));
        }
        for (i, (dot, text)) in self.marks.iter().enumerate() {
            let Some(m) = marks.get(i) else {
                unsafe {
                    let _: () = objc2::msg_send![&**dot, setHidden: true];
                    let _: () = objc2::msg_send![&**text, setHidden: true];
                }
                continue;
            };
            let ink = colour(m.color, 1.);
            unsafe {
                let _: () = objc2::msg_send![&**dot, setBackgroundColor: &*ink];
                let _: () = objc2::msg_send![&**dot, setFrame: NSRect::new(
                    NSPoint::new(m.x - 3., height - m.y - 3.), NSSize::new(6., 6.))];
                let _: () = objc2::msg_send![&**dot, setHidden: false];
                match m.label {
                    Some(baseline) => {
                        // The line box's top sits an ascent above the baseline.
                        let top = baseline - size;
                        let string = objc2_foundation::NSString::from_str(&m.text);
                        let _: () = objc2::msg_send![&**text, setString: &*string];
                        let _: () = objc2::msg_send![&**text, setForegroundColor: &*ink];
                        let _: () = objc2::msg_send![&**text, setContentsScale: scale];
                        let _: () = objc2::msg_send![&**text, setFrame: NSRect::new(
                            NSPoint::new(m.x + 7., height - top - tall), NSSize::new(120., tall))];
                        let _: () = objc2::msg_send![&**text, setHidden: false];
                    }
                    None => {
                        let _: () = objc2::msg_send![&**text, setHidden: true];
                    }
                }
            }
        }
        self.shown = marks.to_vec();
    }
    fn clear_marks(&mut self) {
        if self.shown.is_empty() {
            return;
        }
        for (dot, text) in &self.marks {
            unsafe {
                let _: () = objc2::msg_send![&**dot, setHidden: true];
                let _: () = objc2::msg_send![&**text, setHidden: true];
            }
        }
        self.shown.clear();
    }
    /// Scrolled out of the tracks: the line goes, and its trail with it (the
    /// trail is a sibling layer, so hiding the line alone left the glow
    /// sliding over the layer names during playback).
    pub fn hide(&mut self) {
        unsafe {
            let _: () = objc2::msg_send![&*self.line, setHidden: true];
        }
        self.trail.setHidden(true);
        self.spring.reset();
        self.clear_marks();
    }
}
impl Drop for Playhead {
    fn drop(&mut self) {
        self.trail.removeFromSuperlayer();
        self.head.removeFromSuperlayer();
        for (dot, text) in &self.marks {
            unsafe {
                let _: () = objc2::msg_send![&**dot, removeFromSuperlayer];
                let _: () = objc2::msg_send![&**text, removeFromSuperlayer];
            }
        }
    }
}
/// The timeline's skimmer drawn outside GPUI, as the playhead is: a hairline
/// of text ink down the ruler and tracks and its timecode in a chip on the
/// ruler. A scroll under a resting pointer moves the time under it with no
/// panel draw; the playback loop places it from the timeline's facts.
pub struct Skimmer {
    line: Retained<AnyObject>,
    chip: Retained<AnyObject>,
    text: Retained<AnyObject>,
    /// The timecode shown and its width (measured once per text).
    code: Option<(String, f64)>,
}
impl Skimmer {
    pub fn new(parent: &AnyObject, t: &pm_design::Tokens) -> Self {
        let layer = |class: &objc2::runtime::AnyClass| -> Retained<AnyObject> {
            unsafe { Retained::from_raw(objc2::msg_send![class, new]) }.unwrap()
        };
        let (line, chip, text) = (
            layer(objc2::class!(CALayer)),
            layer(objc2::class!(CALayer)),
            layer(objc2::class!(CATextLayer)),
        );
        let font = objc2_app_kit::NSFont::monospacedDigitSystemFontOfSize_weight(
            t.typography.minimum as f64,
            unsafe { objc2_app_kit::NSFontWeightRegular },
        );
        // CATextLayer.font is a CFTypeRef; NSFont is toll-free bridged to CTFont.
        let font_ref = Retained::as_ptr(&font) as *const std::ffi::c_void;
        unsafe {
            let _: () = objc2::msg_send![&*text, setFont: font_ref];
            let _: () = objc2::msg_send![&*text, setFontSize: t.typography.minimum as f64];
            let _: () = objc2::msg_send![&*text, setAlignmentMode: objc2_quartz_core::kCAAlignmentLeft];
            let _: () = objc2::msg_send![&*chip, addSublayer: &*text];
            let _: () = objc2::msg_send![parent, addSublayer: &*line];
            let _: () = objc2::msg_send![parent, addSublayer: &*chip];
            let _: () = objc2::msg_send![&*line, setHidden: true];
            let _: () = objc2::msg_send![&*chip, setHidden: true];
        }
        Self {
            line,
            chip,
            text,
            code: None,
        }
    }
    /// Places it at `x` (window points) over a timeline body from `top` to
    /// `bottom` (window points, y down) in a parent `height` tall (y up):
    /// the line the body's height, the chip on the ruler right of it.
    #[allow(clippy::too_many_arguments)]
    pub fn place(
        &mut self,
        x: f64,
        top: f64,
        bottom: f64,
        code: &str,
        height: f64,
        scale: f64,
        t: &pm_design::Tokens,
    ) {
        let m = t.metrics;
        let (border, size) = (m.border as f64, t.typography.minimum as f64);
        if self.code.as_ref().is_none_or(|(shown, _)| shown != code) {
            let string = objc2_foundation::NSString::from_str(code);
            unsafe {
                let _: () = objc2::msg_send![&*self.text, setString: &*string];
            }
            let fits: NSSize = unsafe { objc2::msg_send![&*self.text, preferredFrameSize] };
            self.code = Some((code.to_owned(), fits.width.ceil()));
        }
        let width = self.code.as_ref().map_or(0., |(_, w)| *w);
        // As the panel's chip was: the text's line (GPUI's default leading),
        // half a gap each side, a hairline border.
        let line_tall = (size * 1.618).round();
        let chip = NSSize::new(width + m.gap as f64 + 2. * border, line_tall + 2. * border);
        let text_tall = (size * 1.2).ceil();
        let ink = colour(t.colors.text, 0x99 as f64 / 255.);
        unsafe {
            let _: () = objc2::msg_send![&*self.line, setBackgroundColor: &*ink];
            let _: () = objc2::msg_send![&*self.line, setFrame: NSRect::new(
                NSPoint::new(x - border / 2., height - bottom), NSSize::new(border, (bottom - top).max(0.)))];
            let _: () = objc2::msg_send![&*self.chip, setBackgroundColor: &*colour(t.colors.floating, 1.)];
            let _: () = objc2::msg_send![&*self.chip, setBorderColor: &*colour(t.colors.separator, 1.)];
            let _: () = objc2::msg_send![&*self.chip, setBorderWidth: border];
            let _: () = objc2::msg_send![&*self.chip, setCornerRadius: m.control_radius as f64];
            let _: () = objc2::msg_send![&*self.chip, setFrame: NSRect::new(
                NSPoint::new(x + 2. * border, height - (top + border) - chip.height), chip)];
            let _: () = objc2::msg_send![&*self.text, setForegroundColor: &*colour(t.colors.text, 1.)];
            let _: () = objc2::msg_send![&*self.text, setContentsScale: scale];
            let _: () = objc2::msg_send![&*self.text, setFrame: NSRect::new(
                NSPoint::new(border + m.gap as f64 / 2., border + (line_tall - text_tall) / 2.),
                NSSize::new(width, text_tall))];
            let _: () = objc2::msg_send![&*self.line, setHidden: false];
            let _: () = objc2::msg_send![&*self.chip, setHidden: false];
        }
    }
    pub fn hide(&mut self) {
        unsafe {
            let _: () = objc2::msg_send![&*self.line, setHidden: true];
            let _: () = objc2::msg_send![&*self.chip, setHidden: true];
        }
    }
}
impl Drop for Skimmer {
    fn drop(&mut self) {
        unsafe {
            let _: () = objc2::msg_send![&*self.line, removeFromSuperlayer];
            let _: () = objc2::msg_send![&*self.chip, removeFromSuperlayer];
        }
    }
}
/// v1's playhead pin, `d` pt square in a y-up layer: a flat top, straight
/// sides down to its shoulders, then a point at the bottom centre.
fn pin_path(d: f64) -> impl std::ops::Deref<Target = CGMutablePath> {
    let path = CGMutablePath::new();
    let shoulder = d * (5. / 11.);
    unsafe {
        CGMutablePath::move_to_point(Some(&*path), std::ptr::null(), 0., d);
        CGMutablePath::add_line_to_point(Some(&*path), std::ptr::null(), d, d);
        CGMutablePath::add_line_to_point(Some(&*path), std::ptr::null(), d, shoulder);
        CGMutablePath::add_line_to_point(Some(&*path), std::ptr::null(), d / 2., 0.);
        CGMutablePath::add_line_to_point(Some(&*path), std::ptr::null(), 0., shoulder);
    }
    CGMutablePath::close_subpath(Some(&*path));
    path
}
/// A CGColor as the Objective-C object it bridges to (for NSArray).
fn cf_object(c: &CGColor) -> &AnyObject {
    unsafe { &*(c as *const CGColor as *const AnyObject) }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn the_trail_follows_speed_then_settles_to_nothing() {
        let mut trail = Trail::default();
        let start = Instant::now();
        assert_eq!(trail.step(0., start), 0.);
        // 300 pt/s to the right for a second at 60 Hz.
        let mut width = 0.;
        for i in 1..=60 {
            width = trail.step(i as f64 * 5., start + Duration::from_millis(i * 1000 / 60));
        }
        assert!((width - 90.).abs() < 2., "{width}");
        // Stopped: the spring decays to exactly nothing, never overshooting.
        let stop = start + Duration::from_secs(1);
        for i in 1..=60 {
            width = trail.step(300., stop + Duration::from_millis(i * 1000 / 60));
            assert!(width >= 0.);
        }
        assert_eq!(width, 0.);
    }
}
