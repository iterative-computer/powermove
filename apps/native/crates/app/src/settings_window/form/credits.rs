//! Settings › Account's credits this month: a quiet meter of what is left
//! (the accent on a faint track, the warning colour under a tenth) and the period's daily use as thin bars,
//! one slot per day of the period so the month's shape reads at a glance.
//! Hovering a day names it and its credits above its bar. No axes. Every
//! size and colour is a token (`pm_design::Settings`, `Colors`, `Ink`).
use super::{font, regular, st, text_size};
use objc2::{AllocAnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSAccessibility, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSBezierPath, NSColor, NSEvent, NSFontAttributeName, NSForegroundColorAttributeName,
    NSStringDrawing, NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{NSArray, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use pm_design::{Colors, Ink, Scheme};
use std::cell::{Cell, RefCell};

fn colour(rgb: u32, alpha: f64) -> Retained<NSColor> {
    let channel = |shift: u32| ((rgb >> shift) & 255) as f64 / 255.;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), alpha)
}
/// The colours for the appearance the view draws in now.
fn colours(view: &NSView) -> Colors {
    let names = unsafe { [NSAppearanceNameAqua, NSAppearanceNameDarkAqua] };
    let dark = view
        .effectiveAppearance()
        .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&names))
        .is_some_and(|name| &*name == names[1]);
    Colors::of(if dark { Scheme::Dark } else { Scheme::Light })
}
fn fill(rect: NSRect, radius: f64, colour: &NSColor) {
    colour.setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius).fill();
}

#[derive(Default)]
pub(super) struct MeterState {
    used: Cell<u64>,
    included: Cell<u64>,
}

define_class!(
    /// What is left of `included` as a rounded bar over a faint track.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsCreditsMeter"]
    #[ivars = MeterState]
    pub(super) struct Meter;

    impl Meter {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool {
            true
        }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            let c = colours(self);
            let s = st();
            let bounds = self.bounds();
            let h = s.credits_meter as f64;
            let track = NSRect::new(
                NSPoint::new(0., s.credits_air as f64),
                NSSize::new(bounds.size.width, h),
            );
            fill(track, h / 2., &colour(c.text, Ink::new().edge as f64));
            // A fuel gauge: the fill is what is left, draining as credits
            // are spent, and turns to the warning colour under a tenth.
            let (used, included) = (self.ivars().used.get(), self.ivars().included.get());
            let left = included.saturating_sub(used);
            if included > 0 && left > 0 {
                let share = left as f64 / included as f64;
                let ink = if left * 10 < included { c.warning } else { c.accent };
                // Never thinner than its own round ends.
                let width = (bounds.size.width * share).max(h);
                let bar = NSRect::new(track.origin, NSSize::new(width, h));
                fill(bar, h / 2., &colour(ink, 1.));
            }
        }
    }
);

impl Meter {
    pub(super) fn new(mtm: objc2::MainThreadMarker) -> Retained<Self> {
        let view: Retained<Self> = unsafe {
            msg_send![
                super(Self::alloc(mtm).set_ivars(MeterState::default())),
                init
            ]
        };
        view.setTranslatesAutoresizingMaskIntoConstraints(false);
        let s = st();
        view.heightAnchor()
            .constraintEqualToConstant((s.credits_air + s.credits_meter) as f64)
            .setActive(true);
        view
    }
    pub(super) fn set(&self, used: u64, included: u64, label: &str) {
        let state = self.ivars();
        if (state.used.get(), state.included.get()) != (used, included) {
            state.used.set(used);
            state.included.set(included);
            self.setNeedsDisplay(true);
        }
        self.setAccessibilityLabel(Some(&NSString::from_str(label)));
    }
}

#[derive(Default)]
pub(super) struct ChartState {
    /// Each day so far: its name (`Nov 26`) and credits.
    days: RefCell<Vec<(String, u64)>>,
    /// The period's length in days: one slot each, the days to come empty.
    slots: Cell<usize>,
    hovered: Cell<Option<usize>>,
}

define_class!(
    /// The period's daily use as thin bars; the hovered day is drawn in the
    /// accent and named, with its credits, above its bar.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsCreditsChart"]
    #[ivars = ChartState]
    pub(super) struct Chart;

    impl Chart {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool {
            true
        }
        #[unsafe(method(mouseMoved:))]
        fn moved(&self, event: &NSEvent) {
            let at = self.convertPoint_fromView(event.locationInWindow(), None);
            let slot = self.slot_at(at.x);
            if self.ivars().hovered.replace(slot) != slot {
                self.setNeedsDisplay(true);
            }
        }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, _: &NSEvent) {
            if self.ivars().hovered.replace(None).is_some() {
                self.setNeedsDisplay(true);
            }
        }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            let c = colours(self);
            let s = st();
            let state = self.ivars();
            let days = state.days.borrow();
            let width = self.bounds().size.width;
            let (slot, bar) = self.geometry();
            let caption = self.caption_height();
            let top = s.credits_air as f64 + caption;
            let height = s.credits_chart as f64;
            let most = days.iter().map(|(_, c)| *c).max().unwrap_or(0).max(1);
            let hovered = state.hovered.get();
            let quiet = colour(c.secondary, 1.);
            let lit = colour(c.accent, 1.);
            let faint = colour(c.text, Ink::new().edge as f64);
            let caps = s.credits_bar_radius as f64;
            // Every day of the period has a slot: a day spent reaches up
            // from the baseline by its share of the busiest day; a day with
            // none, and every day still to come, is a faint baseline tick.
            let tick = s.credits_bar_gap as f64;
            for i in 0..state.slots.get().max(days.len()) {
                let x = i as f64 * slot + (slot - bar) / 2.;
                let spent = days.get(i).map_or(0, |(_, c)| *c);
                let (h, ink) = if spent == 0 {
                    (tick, if hovered == Some(i) { &lit } else { &faint })
                } else {
                    let h = (height * spent as f64 / most as f64).max(bar.min(height));
                    (h, if hovered == Some(i) { &lit } else { &quiet })
                };
                let rect = NSRect::new(NSPoint::new(x, top + height - h), NSSize::new(bar, h));
                fill(rect, caps.min(h / 2.), ink);
            }
            // Today (the last day read): a small dot under its slot.
            if let Some(today) = days.len().checked_sub(1) {
                let d = s.credits_today as f64;
                let dot = NSRect::new(
                    NSPoint::new((today as f64 + 0.5) * slot - d / 2., top + height + tick),
                    NSSize::new(d, d),
                );
                fill(dot, d / 2., &quiet);
            }
            // The hovered day and its credits, over its bar, kept inside
            // the chart's width.
            if let Some((name, credits)) = hovered.and_then(|i| days.get(i)) {
                let text = NSString::from_str(&format!(
                    "{name} · {} {}",
                    pm_agents::inference::grouped(*credits),
                    if *credits == 1 { "credit" } else { "credits" }
                ));
                let attrs = caption_attributes(&colour(c.text, 1.));
                let size = unsafe { text.sizeWithAttributes(Some(&attrs)) };
                let centre = (hovered.unwrap_or(0) as f64 + 0.5) * slot;
                let x = (centre - size.width / 2.).clamp(0., (width - size.width).max(0.));
                unsafe {
                    text.drawAtPoint_withAttributes(
                        NSPoint::new(x, s.credits_air as f64),
                        Some(&attrs),
                    )
                };
            }
        }
    }
);

fn caption_attributes(
    ink: &NSColor,
) -> Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> {
    let font = font(text_size().xs, regular());
    NSDictionary::<NSString, objc2::runtime::AnyObject>::from_slices(
        &[unsafe { NSFontAttributeName }, unsafe {
            NSForegroundColorAttributeName
        }],
        &[font.as_ref(), ink.as_ref()],
    )
}

impl Chart {
    pub(super) fn new(mtm: objc2::MainThreadMarker) -> Retained<Self> {
        let view: Retained<Self> = unsafe {
            msg_send![
                super(Self::alloc(mtm).set_ivars(ChartState::default())),
                init
            ]
        };
        view.setTranslatesAutoresizingMaskIntoConstraints(false);
        let s = st();
        view.heightAnchor()
            .constraintEqualToConstant(
                s.credits_air as f64
                    + view.caption_height()
                    + s.credits_chart as f64
                    + (s.credits_bar_gap + s.credits_today) as f64,
            )
            .setActive(true);
        let tracking = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::ZERO,
                NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::MouseMoved
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(&view),
                None,
            )
        };
        view.addTrackingArea(&tracking);
        view
    }
    /// The days so far and the period's length in days.
    pub(super) fn set(&self, days: Vec<(String, u64)>, slots: usize, label: &str) {
        let state = self.ivars();
        let slots = slots.max(days.len()).max(1);
        if *state.days.borrow() != days || state.slots.get() != slots {
            *state.days.borrow_mut() = days;
            state.slots.set(slots);
            self.setNeedsDisplay(true);
        }
        self.setAccessibilityLabel(Some(&NSString::from_str(label)));
    }
    /// The hovered caption's line, above the bars.
    fn caption_height(&self) -> f64 {
        let attrs = caption_attributes(&NSColor::labelColor());
        unsafe { NSString::from_str("Dec 25").sizeWithAttributes(Some(&attrs)) }
            .height
            .ceil()
    }
    /// A slot's pitch and its bar's width: thin bars, one per day of the
    /// period, centred in their slots.
    fn geometry(&self) -> (f64, f64) {
        let slots = self.ivars().slots.get().max(1) as f64;
        let s = st();
        let gap = s.credits_bar_gap as f64;
        let slot = (self.bounds().size.width + gap) / slots;
        (slot, (slot - gap).min(s.credits_bar as f64).max(1.))
    }
    /// The day under the pointer, if it is one that has passed.
    fn slot_at(&self, x: f64) -> Option<usize> {
        let (slot, _) = self.geometry();
        let i = (x / slot).floor();
        (i >= 0. && (i as usize) < self.ivars().days.borrow().len()).then_some(i as usize)
    }
}
