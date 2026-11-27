//! The Agent presence chip. The native view and windowless captures share this layout.
use super::*;
use objc2::AnyThread;
use objc2_foundation::{
    NSAttributedString, NSDictionary, NSInsetRect, NSRunLoop, NSRunLoopCommonModes, NSTimer,
};
use pm_design::motion::{Analytic, Preset};
use std::time::Instant;

// Matches the prominent Export capsule in the desktop toolbar.
pub(super) const HEIGHT: f64 = 36.;
const MIN_WIDTH: f64 = 120.;
const MAX_WIDTH: f64 = 300.;
const LEADING: f64 = 10.;
const MARK_GAP: f64 = 7.;
const TRAILING: f64 = 8.;
const BADGE_GAP: f64 = 6.;
const BADGE_PAD: f64 = 4.;
const BADGE_HEIGHT: f64 = 16.;
const UNDO_GAP: f64 = 10.;
const UNDO_PAD: f64 = 5.;
const UNDO_HEIGHT: f64 = 22.;
const ACTION_WIDTH: f64 = 22.;
const ACTION_GAP: f64 = 2.;
const ACTIONS_WIDTH: f64 = 2. * ACTION_WIDTH + ACTION_GAP;
const HOVER_SECONDS: f64 = 0.120;
const SIZE_SECONDS: f64 = 0.180;
pub(super) const ACTIONS: [&str; 2] = ["Ⅱ", "■"];

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn centered(x: f64, w: f64, h: f64) -> NSRect {
    rect(x, (HEIGHT - h) / 2., w, h)
}

fn capsule(rect: NSRect, radius: f64) -> Retained<NSBezierPath> {
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius)
}

pub(super) fn title(text: &str, size: f64, color: &NSColor) -> Retained<NSAttributedString> {
    let font = NSFont::systemFontOfSize(size);
    let attrs = NSDictionary::from_slices(
        &[unsafe { NSFontAttributeName }, unsafe {
            NSForegroundColorAttributeName
        }],
        &[&*font as &AnyObject, color as &AnyObject],
    );
    unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &NSString::from_str(text),
            Some(&attrs),
        )
    }
}

fn copy(status: &AgentStatus) -> &str {
    if !status.working {
        status.undo_feedback.unwrap_or("Done")
    } else if status.paused {
        "Paused · ⇧⌘. to resume"
    } else if status.held {
        "Waiting for you to finish"
    } else {
        status.progress.as_deref().unwrap_or("Agent working")
    }
}

struct Content {
    text: String,
    working: bool,
    badge: Option<Retained<NSAttributedString>>,
    undo: Option<Retained<NSAttributedString>>,
}

impl Content {
    fn new(status: &AgentStatus, t: &Tokens) -> Self {
        let count = status.additional_runs;
        Self {
            working: status.working,
            text: copy(status)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            badge: (status.working && count > 0)
                .then(|| title(&format!("+{count}"), 10., &NSColor::secondaryLabelColor())),
            undo: (!status.working
                && status.undo_available
                && status.undo_feedback != Some(AgentStatus::UNDOING))
                .then(|| title("Undo", 13., &native_color(t.colors.accent_text))),
        }
    }

    fn text_x(&self) -> f64 {
        LEADING + if self.working { 8. } else { 10. } + MARK_GAP
    }

    fn text_width(&self, text: &str) -> f64 {
        title(text, 13., &NSColor::controlTextColor())
            .size()
            .width
            .ceil()
    }

    fn badge_width(&self) -> f64 {
        self.badge
            .as_ref()
            .map_or(0., |badge| badge.size().width.ceil() + 2. * BADGE_PAD)
    }

    fn undo_width(&self) -> f64 {
        self.undo
            .as_ref()
            .map_or(0., |undo| undo.size().width.ceil() + 2. * UNDO_PAD)
    }

    fn decorations(&self) -> f64 {
        self.badge
            .as_ref()
            .map_or(0., |_| BADGE_GAP + self.badge_width())
            + self
                .undo
                .as_ref()
                .map_or(0., |_| UNDO_GAP + self.undo_width())
    }

    fn preferred_width(&self) -> f64 {
        let fixed = self.text_x() + self.decorations() + TRAILING;
        let natural = fixed + self.text_width(&self.text);
        // Very short multi-run titles still need room for the badge and an
        // ellipsis when controls appear. Don't reserve that column otherwise.
        let hover_min = if self.badge.is_some() {
            fixed + self.text_width("…") + BADGE_GAP + ACTIONS_WIDTH
        } else {
            MIN_WIDTH
        };
        natural.max(hover_min).clamp(MIN_WIDTH, MAX_WIDTH)
    }

    fn layout(&self, width: f64, hover: bool) -> Layout {
        let actions = (hover && self.working)
            .then(|| centered(width - TRAILING - ACTIONS_WIDTH, ACTIONS_WIDTH, HEIGHT));
        let text_right = actions.map_or(width - TRAILING, |r| r.origin.x - BADGE_GAP);
        let x = self.text_x();
        let budget = (text_right - x - self.decorations()).max(0.);
        let fitted = crate::agent_launcher::fit_presence_title(&self.text, budget, |text| {
            self.text_width(text)
        });
        let title = title(&fitted, 13., &NSColor::controlTextColor());
        let end = x + title.size().width.ceil();
        Layout {
            width,
            title,
            text_x: x,
            badge: self.badge.as_ref().map(|badge| {
                (
                    centered(end + BADGE_GAP, self.badge_width(), BADGE_HEIGHT),
                    badge.clone(),
                )
            }),
            undo: self.undo.as_ref().map(|undo| {
                (
                    centered(end + UNDO_GAP, self.undo_width(), UNDO_HEIGHT),
                    undo.clone(),
                )
            }),
            actions,
        }
    }
}

pub(super) struct Layout {
    pub(super) width: f64,
    pub(super) title: Retained<NSAttributedString>,
    pub(super) text_x: f64,
    pub(super) badge: Option<(NSRect, Retained<NSAttributedString>)>,
    pub(super) undo: Option<(NSRect, Retained<NSAttributedString>)>,
    pub(super) actions: Option<NSRect>,
}

#[cfg(test)]
pub(super) fn layout(status: &AgentStatus, t: &Tokens, hovered: bool) -> Layout {
    let content = Content::new(status, t);
    content.layout(content.preferred_width(), hovered)
}

fn draw_centered(text: &NSAttributedString, r: NSRect) {
    let s = text.size();
    text.drawAtPoint(NSPoint::new(
        r.origin.x + (r.size.width - s.width) / 2.,
        r.origin.y + (r.size.height - s.height) / 2.,
    ));
}

impl Layout {
    fn draw(&self, t: &Tokens, working: bool) {
        native_color(t.colors.field).setFill();
        capsule(rect(0., 0., self.width, HEIGHT), HEIGHT / 2.).fill();
        if working {
            draw_centered(
                &self.title,
                centered(self.text_x, self.title.size().width, HEIGHT),
            );
        }
        if let Some((r, badge)) = &self.badge {
            // Inset the stroke so the measured capsule includes the full 1 pt.
            let border = NSInsetRect(*r, 0.5, 0.5);
            let shape = capsule(border, border.size.height / 2.);
            NSColor::secondaryLabelColor()
                .colorWithAlphaComponent(0.55)
                .setStroke();
            shape.setLineWidth(1.);
            shape.stroke();
            draw_centered(badge, *r);
        }
    }

    #[cfg(test)]
    pub(super) fn draw_fixture(&self, status: &AgentStatus, t: &Tokens, undo_hot: bool) {
        self.draw(t, status.working);
        if !status.working {
            draw_centered(
                &self.title,
                centered(self.text_x, self.title.size().width, HEIGHT),
            );
        }
        let (marker, image) = marker(status, 0., t);
        image.drawInRect(marker);
        if let Some((r, undo)) = &self.undo {
            if undo_hot {
                native_color(t.colors.accent)
                    .colorWithAlphaComponent(0.16)
                    .setFill();
                capsule(*r, r.size.height / 2.).fill();
            }
            draw_centered(undo, *r);
        }
        if let Some(r) = self.actions {
            for (index, glyph) in ACTIONS.iter().enumerate() {
                draw_centered(
                    &title(glyph, 13., &NSColor::controlTextColor()),
                    centered(
                        r.origin.x + index as f64 * (ACTION_WIDTH + ACTION_GAP),
                        ACTION_WIDTH,
                        HEIGHT,
                    ),
                );
            }
        }
    }
}

fn marker(status: &AgentStatus, phase: f64, t: &Tokens) -> (NSRect, Retained<NSImage>) {
    let working = status.working;
    let paused = status.paused;
    let feedback = status.undo_feedback;
    let glow = working && !paused && !status.held && !status.attention;
    let alpha = if t.accessibility.reduce_motion {
        0.35
    } else {
        0.35 + 0.15 * (phase * std::f64::consts::PI).sin()
    };
    let accent = t.colors.accent;
    let size = 18.;
    let center = NSPoint::new(9., 9.);
    let draw = block2::RcBlock::new(move |_: NSRect| -> objc2::runtime::Bool {
        if working {
            let color = if paused {
                NSColor::secondaryLabelColor()
            } else {
                native_color(accent)
            };
            if glow
                && let Some(soft) = NSGradient::initWithStartingColor_endingColor(
                    NSGradient::alloc(),
                    &color.colorWithAlphaComponent(alpha),
                    &color.colorWithAlphaComponent(0.),
                )
            {
                soft.drawFromCenter_radius_toCenter_radius_options(
                    center,
                    4.,
                    center,
                    9.,
                    NSGradientDrawingOptions::empty(),
                );
            }
            color.setFill();
            NSBezierPath::bezierPathWithOvalInRect(rect(5., 5., 8., 8.)).fill();
        } else if feedback.is_none() || feedback == Some(AgentStatus::UNDONE) {
            NSColor::secondaryLabelColor().setStroke();
            let check = NSBezierPath::bezierPath();
            check.setLineWidth(1.5);
            check.setLineCapStyle(NSLineCapStyle::Round);
            check.setLineJoinStyle(NSLineJoinStyle::Round);
            check.moveToPoint(NSPoint::new(4.75, 8.6));
            check.lineToPoint(NSPoint::new(7.7, 5.8));
            check.lineToPoint(NSPoint::new(13.25, 11.9));
            check.stroke();
        } else {
            NSColor::secondaryLabelColor().setFill();
            NSBezierPath::bezierPathWithOvalInRect(rect(6., 6., 6., 6.)).fill();
        }
        objc2::runtime::Bool::YES
    });
    (
        centered(LEADING + if working { 4. } else { 5. } - 9., size, size),
        NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(size, size), false, &draw),
    )
}

fn tracking(view: &NSView) {
    for area in view.trackingAreas().to_vec() {
        view.removeTrackingArea(&area);
    }
    let area = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            view.bounds(),
            NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::ActiveInActiveApp
                | NSTrackingAreaOptions::InVisibleRect,
            Some(view),
            None,
        )
    };
    view.addTrackingArea(&area);
}

struct UndoState {
    hovered: Cell<bool>,
    pressed: Cell<bool>,
    accent: Cell<u32>,
    reduce_motion: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSButton))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMCoeditUndo"]
    #[ivars = UndoState]
    struct Undo;
    impl Undo {
        #[unsafe(method(updateTrackingAreas))]
        fn tracking(&self) {
            unsafe { let _: () = msg_send![super(self), updateTrackingAreas]; }
            tracking(self);
        }
        #[unsafe(method(mouseEntered:))]
        fn entered(&self, _: &NSEvent) {
            self.ivars().hovered.set(true);
            self.update_fill();
        }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, _: &NSEvent) {
            self.ivars().hovered.set(false);
            self.update_fill();
        }
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool { false }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _: Option<&NSEvent>) -> bool { true }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _: &NSEvent) {
            if !self.isEnabled() {
                return;
            }
            let Some(window) = self.window() else {
                return;
            };
            self.ivars().pressed.set(true);
            self.update_fill();
            self.displayIfNeeded();
            // The entire visible capsule is clickable, including the padding
            // outside the borderless NSButtonCell's title.
            let mask = NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp;
            let deadline = NSDate::distantFuture();
            while let Some(event) = window.nextEventMatchingMask_untilDate_inMode_dequeue(
                mask,
                Some(&deadline),
                unsafe { NSEventTrackingRunLoopMode },
                true,
            ) {
                let point = self.convertPoint_fromView(event.locationInWindow(), None);
                let inside = self.mouse_inRect(point, self.bounds());
                if event.r#type() == NSEventType::LeftMouseUp {
                    self.ivars().hovered.set(inside);
                    self.ivars().pressed.set(false);
                    self.update_fill();
                    if inside && self.isEnabled() {
                        unsafe { self.sendAction_to(self.action(), self.target().as_deref()) };
                    }
                    return;
                }
                self.ivars().hovered.set(inside);
                self.ivars().pressed.set(inside);
                self.update_fill();
                self.displayIfNeeded();
            }
            self.ivars().pressed.set(false);
            self.update_fill();
        }
        #[unsafe(method(highlight:))]
        fn highlight(&self, highlighted: objc2::runtime::Bool) {
            unsafe { let _: () = msg_send![super(self), highlight: highlighted]; }
            self.ivars().pressed.set(highlighted.as_bool());
            self.update_fill();
        }
    }
);

impl Undo {
    fn update_fill(&self) {
        if let Some(layer) = self.layer() {
            let state = self.ivars();
            let alpha = if state.pressed.get() {
                0.28
            } else if state.hovered.get() {
                0.16
            } else {
                0.
            };
            let color = native_color(state.accent.get()).colorWithAlphaComponent(alpha);
            let scale = if state.pressed.get() && !state.reduce_motion.get() { 0.96 } else { 1. };
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            layer.setBackgroundColor(Some(&color.CGColor()));
            layer.setTransform(objc2_quartz_core::CATransform3D::new_scale(scale, scale, 1.));
            CATransaction::commit();
        }
    }
}

pub(super) struct Views {
    tokens: RefCell<Tokens>,
    status: RefCell<Option<AgentStatus>>,
    content: RefCell<Content>,
    layout: RefCell<Layout>,
    width: Cell<f64>,
    motion: Cell<Option<(Instant, Analytic)>>,
    hovered: Cell<bool>,
    marker: Retained<NSImageView>,
    steer: Retained<NSButton>,
    done: Retained<NSTextField>,
    undo: Retained<Undo>,
    actions: Retained<NSView>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMCoeditPresence"]
    #[ivars = Views]
    pub(super) struct CoeditPresence;
    impl CoeditPresence {
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool { false }

        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic(&self) -> NSSize {
            NSSize::new(self.ivars().width.get(), HEIGHT)
        }
        #[unsafe(method(layout))]
        fn layout(&self) {
            unsafe { let _: () = msg_send![super(self), layout]; }
            let width = self.bounds().size.width;
            if width > 0. && width != self.ivars().layout.borrow().width {
                self.refresh(width);
            }
        }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            let v = self.ivars();
            let working = v.status.borrow().as_ref().is_some_and(|status| status.working);
            v.layout.borrow().draw(&v.tokens.borrow(), working);
        }
        #[unsafe(method(updateTrackingAreas))]
        fn tracking(&self) {
            unsafe { let _: () = msg_send![super(self), updateTrackingAreas]; }
            tracking(self);
        }
        #[unsafe(method(mouseEntered:))]
        fn entered(&self, _: &NSEvent) { self.set_hovered(true); }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, _: &NSEvent) { self.set_hovered(false); }
    }
);

impl CoeditPresence {
    pub(super) fn new(mtm: MainThreadMarker, target: &Target, t: &Tokens) -> Retained<Self> {
        let configure = |button: &NSButton, tag| {
            button.setTag(tag);
            button.setBordered(false);
            unsafe {
                button.setTarget(Some(target));
                button.setAction(Some(sel!(coedit:)));
            }
        };
        let button = |tag, text: &str| {
            let b = unsafe {
                NSButton::buttonWithTitle_target_action(&NSString::from_str(text), None, None, mtm)
            };
            configure(&b, tag);
            b
        };
        let steer = button(0, "");
        steer.setTransparent(true);
        let done = NSTextField::labelWithString(&NSString::from_str("Done"), mtm);
        done.setFont(Some(&NSFont::systemFontOfSize(13.)));
        done.setTextColor(Some(&NSColor::controlTextColor()));
        let undo: Retained<Undo> = unsafe {
            msg_send![super(Undo::alloc(mtm).set_ivars(UndoState {
                hovered: Cell::new(false), pressed: Cell::new(false), accent: Cell::new(t.colors.accent),
                reduce_motion: Cell::new(t.accessibility.reduce_motion),
            })), initWithFrame: NSRect::ZERO]
        };
        configure(&undo, 1);
        undo.setButtonType(NSButtonType::MomentaryPushIn);
        undo.setAlignment(NSTextAlignment::Center);
        undo.setWantsLayer(true);
        if let Some(layer) = undo.layer() {
            layer.setCornerRadius(UNDO_HEIGHT / 2.);
        }
        undo.setAccessibilityLabel(Some(&NSString::from_str("Undo Agent Turn")));
        let actions = NSView::new(mtm);
        for (i, label) in ["Pause or Resume Agent", "Stop Agent"].iter().enumerate() {
            let b = button(i as isize + 2, ACTIONS[i]);
            let label = NSString::from_str(label);
            b.setFrame(centered(
                i as f64 * (ACTION_WIDTH + ACTION_GAP),
                ACTION_WIDTH,
                HEIGHT,
            ));
            b.setToolTip(Some(&label));
            b.setAccessibilityLabel(Some(&label));
            actions.addSubview(&b);
        }
        let marker = NSImageView::new(mtm);
        marker.setAccessibilityElement(false);
        let content = Content::new(&AgentStatus::default(), t);
        let view: Retained<Self> = unsafe {
            msg_send![super(Self::alloc(mtm).set_ivars(Views {
                tokens: RefCell::new(t.clone()), status: RefCell::new(None),
                layout: RefCell::new(content.layout(MIN_WIDTH,false)), content: RefCell::new(content),
                width: Cell::new(MIN_WIDTH), motion: Cell::new(None), hovered: Cell::new(false),
                marker, steer, done, undo, actions,
            })), initWithFrame: rect(0.,0.,MIN_WIDTH,HEIGHT)]
        };
        view.setTranslatesAutoresizingMaskIntoConstraints(false);
        view.setContentHuggingPriority_forOrientation(
            750.,
            NSLayoutConstraintOrientation::Horizontal,
        );
        let v = view.ivars();
        for child in [
            &*v.marker as &NSView,
            &*v.steer,
            &*v.done,
            &*v.undo,
            &*v.actions,
        ] {
            view.addSubview(child);
        }
        v.done.setHidden(true);
        v.undo.setHidden(true);
        v.actions.setHidden(true);
        view
    }

    pub(super) fn set_status(&self, status: &AgentStatus, t: &Tokens, force: bool) {
        let v = self.ivars();
        let mut old = v.status.borrow_mut();
        let first = old.is_none();
        let update = force
            || old.as_ref().is_none_or(|before| {
                before.working != status.working
                    || before.additional_runs != status.additional_runs
                    || before.undo_available != status.undo_available
                    || copy(before) != copy(status)
            });
        *old = Some(status.clone());
        drop(old);
        if !update {
            return;
        }
        if force {
            *v.tokens.borrow_mut() = t.clone();
            v.undo.ivars().accent.set(t.colors.accent);
            v.undo.ivars().reduce_motion.set(t.accessibility.reduce_motion);
            v.undo.update_fill();
        }
        let content = Content::new(status, t);
        let preferred = content.preferred_width();
        let label = if status.additional_runs > 0 {
            format!(
                "{}, {} additional active runs",
                content.text, status.additional_runs
            )
        } else {
            content.text.clone()
        };
        v.steer
            .setAccessibilityLabel(Some(&NSString::from_str(&label)));
        *v.content.borrow_mut() = content;
        if v.content.borrow().undo.is_none() {
            v.undo.ivars().hovered.set(false);
            v.undo.ivars().pressed.set(false);
            v.undo.update_fill();
        }
        self.resize_to(preferred, first || t.accessibility.reduce_motion);
        self.refresh(self.available_width());
    }

    pub(super) fn paint_marker(&self, status: &AgentStatus, phase: f64, t: &Tokens) {
        let (frame, image) = marker(status, phase, t);
        let view = &self.ivars().marker;
        if view.frame() != frame {
            view.setFrame(frame);
        }
        view.setImage(Some(&image));
    }

    fn available_width(&self) -> f64 {
        let bounds = self.bounds().size.width;
        if unsafe { self.superview() }.is_some() && bounds > 0. {
            bounds
        } else {
            self.ivars().width.get()
        }
    }

    fn refresh(&self, width: f64) {
        let v = self.ivars();
        let content = v.content.borrow();
        let next = content.layout(width, v.hovered.get());
        let working = content.working;
        v.steer.setHidden(!working);
        let end = next.actions.map_or(width, |r| r.origin.x);
        // The native button covers the marker as well as the label (Steer).
        v.steer.setFrame(centered(0., end, HEIGHT));
        v.done.setHidden(working);
        v.undo.setHidden(next.undo.is_none());
        v.undo.setEnabled(next.undo.is_some());
        if !working {
            v.done.setStringValue(&next.title.string());
            // The plain NSTextField label has a 2 pt internal text inset.
            v.done.setFrame(centered(
                next.text_x - 2.,
                next.title.size().width + 5.,
                v.done.fittingSize().height,
            ));
        }
        if let Some((r, title)) = &next.undo {
            v.undo.setFrame(*r);
            v.undo.setAttributedTitle(title);
        }
        let show = next.actions.is_some();
        if let Some(r) = next.actions {
            v.actions.setFrame(r);
        }
        if show != !v.actions.isHidden() {
            v.actions.setHidden(!show);
            if show {
                if v.tokens.borrow().accessibility.reduce_motion {
                    v.actions.setAlphaValue(1.);
                } else {
                    v.actions.setAlphaValue(0.);
                    fade(&v.actions, 1., HOVER_SECONDS);
                }
            }
        }
        *v.layout.borrow_mut() = next;
        self.setNeedsDisplay(true);
    }

    fn set_hovered(&self, hovered: bool) {
        if self.ivars().hovered.replace(hovered) != hovered {
            self.refresh(self.available_width());
        }
    }

    fn apply_width(&self, width: f64) {
        if self.ivars().width.replace(width) != width {
            self.invalidateIntrinsicContentSize();
            self.setNeedsLayout(true);
            // AppKit owns this view hierarchy on the main thread.
            if let Some(parent) = unsafe { self.superview() } {
                parent.setNeedsLayout(true);
            } else {
                self.setFrameSize(NSSize::new(width, HEIGHT));
            }
        }
    }

    fn resize_to(&self, target: f64, snap: bool) {
        let v = self.ivars();
        if snap {
            v.motion.set(None);
            self.apply_width(target);
            return;
        }
        let previous = v.motion.get();
        if previous.map_or(v.width.get(), |(_, spring)| spring.target) == target {
            return;
        }
        let velocity = previous.map_or(0., |(start, spring)| {
            spring.sample(start.elapsed().as_secs_f64() * 2.).1
        });
        let spring = Preset::Layout.spring(&v.tokens.borrow().motion);
        v.motion.set(Some((
            Instant::now(),
            Analytic {
                from: v.width.get(),
                velocity,
                target,
                mass: 1.,
                stiffness: spring.stiffness as f64,
                damping: spring.damping as f64,
            },
        )));
        if previous.is_some() {
            return;
        }
        let weak = objc2::rc::Weak::new(self);
        let tick = block2::RcBlock::new(move |timer: std::ptr::NonNull<NSTimer>| {
            if !weak.load().is_some_and(|view| view.step_width()) {
                unsafe {
                    timer.as_ref().invalidate();
                }
            }
        });
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(1. / 60., true, &tick) };
        unsafe {
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        }
    }

    fn step_width(&self) -> bool {
        let Some((start, spring)) = self.ivars().motion.get() else {
            return false;
        };
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed >= SIZE_SECONDS {
            self.ivars().motion.set(None);
            self.apply_width(spring.target);
            return false;
        }
        self.apply_width(spring.sample(2. * elapsed).0.clamp(MIN_WIDTH, MAX_WIDTH));
        true
    }
}
