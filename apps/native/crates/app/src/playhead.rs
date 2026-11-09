//! The timeline playhead's CALayers, driven by the display link (S3: no GPUI
//! repaint while playing): an `accent` line with a `panel` halo so it reads
//! over clips of any colour, a pin head, and a motion trail whose width
//! follows the head's measured speed through a critically damped spring
//! (v1's playhead). The spring runs only while the head moves.
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_core_graphics::{CGColor, CGMutablePath};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CAGradientLayer, CAShapeLayer, CATransaction};
use std::time::Instant;

/// Head size, points: as wide as the language's playhead head, a little taller
/// for the point.
pub const HEAD: (f64, f64) = (11., 14.);
/// v1's trail: width per point/second of head speed, spring response (s),
/// widest trail (pt) and its opacity at the head.
const TRAIL_GAIN: f64 = 0.3;
const TRAIL_RESPONSE: f64 = 0.15;
const TRAIL_MAX: f64 = 150.;
const TRAIL_ALPHA: f64 = 0.22;

fn colour(rgb: u32, alpha: f64) -> impl std::ops::Deref<Target = CGColor> {
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
        // The pin, y up: a flat top with rounded shoulders narrowing to a point.
        let (w, h) = HEAD;
        let path = CGMutablePath::new();
        let p = Some(&*path);
        let null = std::ptr::null();
        let (r, shoulder) = (3., h * 0.45);
        unsafe {
            CGMutablePath::move_to_point(p, null, 0., shoulder);
            CGMutablePath::add_line_to_point(p, null, 0., h - r);
            CGMutablePath::add_arc_to_point(p, null, 0., h, r, h, r);
            CGMutablePath::add_line_to_point(p, null, w - r, h);
            CGMutablePath::add_arc_to_point(p, null, w, h, w, h - r, r);
            CGMutablePath::add_line_to_point(p, null, w, shoulder);
            CGMutablePath::add_arc_to_point(p, null, w / 2., 0., 0., shoulder, 2.);
        }
        CGMutablePath::close_subpath(p);
        head.setPath(Some(&path));
        head.setLineWidth(1.);
        Self {
            line,
            head,
            trail,
            spring: Trail::default(),
        }
    }
    /// Colours follow the tokens (a theme or appearance change).
    pub fn style(&self, t: &pm_design::Tokens, scale: f64) {
        let accent = colour(t.colors.accent, 1.);
        let panel = colour(t.colors.panel, 1.);
        unsafe {
            let _: () = objc2::msg_send![&*self.line, setBackgroundColor: &*accent];
            let _: () = objc2::msg_send![&*self.line, setShadowColor: &*panel];
            let _: () = objc2::msg_send![&*self.line, setShadowOpacity: 0.9f32];
            let _: () = objc2::msg_send![&*self.line, setShadowRadius: t.metrics.border as f64];
            let _: () = objc2::msg_send![&*self.line, setShadowOffset: NSSize::new(0., 0.)];
        }
        self.head.setFillColor(Some(&accent));
        self.head.setStrokeColor(Some(&panel));
        self.head.setContentsScale(scale);
        let clear = colour(t.colors.accent, 0.);
        let glow = colour(t.colors.accent, TRAIL_ALPHA);
        let colours: [&AnyObject; 2] = [cf_object(&clear), cf_object(&glow)];
        let array = NSArray::from_slice(&colours);
        unsafe { self.trail.setColors(Some(&array)) };
    }
    /// Place the line (`line` pt wide, `tall` pt from `bottom` up, at `x` in
    /// the parent) and the trail over the tracks below the ruler.
    #[allow(clippy::too_many_arguments)]
    pub fn place(
        &mut self,
        x: f64,
        bottom: f64,
        line: f64,
        tall: f64,
        ruler: f64,
        moving: bool,
        t: &pm_design::Tokens,
    ) {
        unsafe {
            let _: () = objc2::msg_send![&*self.line, setFrame: NSRect::new(
                NSPoint::new(x - line / 2., bottom), NSSize::new(line, tall))];
        }
        let (w, h) = HEAD;
        self.head.setFrame(NSRect::new(
            NSPoint::new((line - w) / 2., tall - h),
            NSSize::new(w, h),
        ));
        let width = if moving && !t.accessibility.reduce_motion {
            self.spring.step(x, Instant::now())
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
        let left = if width > 0. { x - span } else { x };
        self.trail.setFrame(NSRect::new(
            NSPoint::new(left, bottom),
            NSSize::new(span, (tall - ruler).max(0.)),
        ));
        self.trail.setOpacity(1.);
        self.trail.setHidden(false);
    }
}
impl Drop for Playhead {
    fn drop(&mut self) {
        self.trail.removeFromSuperlayer();
        self.head.removeFromSuperlayer();
    }
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
