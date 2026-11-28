//! Settings › Account's plans on Free, after the website's pricing cards:
//! Free, the plan the account is on, as a quiet Settings card; each plan the
//! backend offers as the highlighted card, dark in both appearances with the
//! site's warm light rising from its foot. Each card, top down, on one
//! padding: the name; the price large with a quiet period; one line saying
//! who it is for; its action (a real push button, "Get Aerolite", which
//! starts Checkout for that plan; Free says "Current plan"); a hairline;
//! the features, each a check mark, a line and a quieter detail. Every size
//! and colour is a token.
//!
//! An option's label is its lines: name, price, period, note, action (a
//! button's title, or `=` and a quiet status), then one feature a line
//! (`title\tdetail`). A plan's value is its index; the button sends it.
use super::ring::srgb;
use super::{font, regular, space, st, text_size};
use objc2::{
    AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::NSObject,
};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityGroupRole, NSAppearanceCustomization, NSAppearanceNameAqua,
    NSAppearanceNameDarkAqua, NSBezelStyle, NSBezierPath, NSButton, NSColor, NSControlSize, NSFont,
    NSFontAttributeName, NSForegroundColorAttributeName, NSGradient, NSLineCapStyle,
    NSLineJoinStyle, NSStackView, NSStackViewDistribution, NSStringDrawing, NSStringDrawingOptions,
    NSStringNSExtendedStringDrawing, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{
    NSAlignmentOptions, NSArray, NSAttributedString, NSDictionary, NSEdgeInsets, NSObjectProtocol,
    NSPoint, NSRect, NSSize, NSString,
};
use pm_design::{Colors, Ink, Scheme};
use serde_json::{Value, json};
use std::{cell::OnceCell, rc::Rc};

fn attributes(
    font: &NSFont,
    ink: &NSColor,
) -> Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> {
    NSDictionary::<NSString, objc2::runtime::AnyObject>::from_slices(
        &[unsafe { NSFontAttributeName }, unsafe {
            NSForegroundColorAttributeName
        }],
        &[font.as_ref(), ink.as_ref()],
    )
}
/// Inside a card, the same on every side (matches Settings' section rhythm).
fn padding() -> f64 {
    st().section_gap as f64
}
/// The cards' width as laid out: the page's column (the window less its
/// sidebar and the page's insets, at most `settings.column`), less the
/// group card's row inset on both sides and the gap between two cards,
/// halved.
fn card_width() -> f64 {
    let s = st();
    let page = (s.window_width - s.sidebar - 2. * s.page_inset).min(s.column);
    ((page - 2. * s.row_inset - space().lg) / 2.) as f64
}

/// One plan, read from its option's lines.
#[derive(Clone, Debug, Default, PartialEq)]
struct Plan {
    name: String,
    price: String,
    period: String,
    note: String,
    /// A button's title, or (`=…`) a quiet status where the button stands.
    action: Action,
    features: Vec<(String, String)>,
}
#[derive(Clone, Debug, Default, PartialEq)]
enum Action {
    #[default]
    None,
    Button(String),
    Status(String),
}
impl Plan {
    fn parse(option: &str) -> Self {
        let lines: Vec<&str> = option.lines().collect();
        let at = |i: usize| lines.get(i).copied().unwrap_or_default().to_owned();
        let action = match at(4) {
            a if a.is_empty() => Action::None,
            a => match a.strip_prefix('=') {
                Some(status) => Action::Status(status.to_owned()),
                None => Action::Button(a),
            },
        };
        let features = lines
            .iter()
            .skip(5)
            .map(|line| {
                let (title, detail) = line.split_once('\t').unwrap_or((line, ""));
                (title.to_owned(), detail.to_owned())
            })
            .collect();
        Self {
            name: at(0),
            price: at(1),
            period: at(2),
            note: at(3),
            action,
            features,
        }
    }
    /// The highlighted card: a plan to buy.
    fn offered(&self) -> bool {
        matches!(self.action, Action::Button(_))
    }
    /// What VoiceOver reads: every line, in order.
    fn spoken(&self) -> String {
        let price = format!("{} {}", self.price, self.period).trim().to_owned();
        let mut parts = vec![self.name.clone(), price];
        parts.push(self.note.clone());
        if let Action::Status(status) = &self.action {
            parts.push(status.clone());
        }
        for (title, detail) in &self.features {
            parts.push(title.clone());
            parts.push(detail.clone());
        }
        parts.retain(|p| !p.trim().is_empty());
        parts.join(", ")
    }
}

/// The card's type: name, price, period, note, feature, its detail.
struct Type {
    name: Retained<NSFont>,
    price: Retained<NSFont>,
    period: Retained<NSFont>,
    note: Retained<NSFont>,
    feature: Retained<NSFont>,
    detail: Retained<NSFont>,
}
impl Type {
    fn new() -> Self {
        let (t, s) = (text_size(), st());
        // Settings' own scale: a row's title (base) and its secondary line
        // (xs); the name at a group heading's weight; only the price large.
        Self {
            name: font(t.base, s.heading_weight),
            price: font(t.x4l, s.heading_weight),
            period: font(t.xs, regular()),
            note: font(t.xs, regular()),
            feature: font(t.base, regular()),
            detail: font(t.xs, regular()),
        }
    }
}
fn line_height(font: &NSFont) -> f64 {
    let attrs = attributes(font, &NSColor::labelColor());
    unsafe { NSString::from_str("Ag").sizeWithAttributes(Some(&attrs)) }
        .height
        .ceil()
}
/// `text` wrapped at `width` in `font`: its height.
fn wrapped(text: &str, font: &NSFont, width: f64) -> f64 {
    if text.is_empty() {
        return 0.;
    }
    let attrs = attributes(font, &NSColor::labelColor());
    let rect = unsafe {
        NSString::from_str(text).boundingRectWithSize_options_attributes_context(
            NSSize::new(width.max(1.), f64::MAX),
            NSStringDrawingOptions::UsesLineFragmentOrigin
                | NSStringDrawingOptions::UsesFontLeading,
            Some(&attrs),
            None,
        )
    };
    rect.size.height.ceil()
}
/// The action's row: a regular push button's height.
fn action_height() -> f64 {
    st().row as f64 - 2. * st().row_padding as f64
}
/// The cap height trimmed off the top, as `text-box: trim-both cap` does.
fn top_trim(ty: &Type) -> f64 {
    ty.name.ascender() - ty.name.capHeight()
}

/// Where each part of a card stands, from its top (padding included), for a
/// text column `width` wide.
struct Layout {
    price: f64,
    note: f64,
    action: f64,
    hairline: f64,
    features: Vec<(f64, f64)>,
    height: f64,
}
fn layout(ty: &Type, plan: &Plan, width: f64) -> Layout {
    let (p, sm, md, lg) = (
        padding(),
        space().sm as f64,
        space().md as f64,
        space().lg as f64,
    );
    let mut y = p - top_trim(ty) + line_height(&ty.name) + sm;
    let price = y;
    y += line_height(&ty.price) + space().xs as f64;
    let note = y;
    y += wrapped(&plan.note, &ty.note, width).max(line_height(&ty.note)) + lg;
    let action = y;
    y += action_height() + lg;
    let hairline = y.round();
    y += lg;
    let text = width - line_height(&ty.feature) - sm;
    let mut features = vec![];
    for (_, detail) in &plan.features {
        let below = y + line_height(&ty.feature) + space().hair as f64 / 2.;
        features.push((y, below));
        y = below + wrapped(detail, &ty.detail, text) + md;
    }
    let height = y - if plan.features.is_empty() { lg } else { md } + p;
    Layout {
        price,
        note,
        action,
        hairline,
        features,
        height: height.ceil(),
    }
}

pub(super) struct CardState {
    plan: Plan,
    all_plans: Rc<Vec<Plan>>,
    height: OnceCell<Retained<objc2_app_kit::NSLayoutConstraint>>,
    radius: f64,
}

define_class!(
    /// One plan, drawn as a card; its action is a real button inside it.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsPlanCard"]
    #[ivars = CardState]
    pub(super) struct Card;

    impl Card {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool {
            true
        }
        /// The action's button stands where the card's layout puts it, at the
        /// card's own width, and the card's height tracks its real laid-out
        /// width so wrapped feature lines never crowd the bottom edge.
        #[unsafe(method(layout))]
        fn lay_out(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            let bounds = self.bounds();
            let p = padding();
            let width = bounds.size.width - 2. * p;
            if width <= 1. {
                return;
            }
            let ty = Type::new();
            let needed = self
                .ivars()
                .all_plans
                .iter()
                .map(|plan| layout(&ty, plan, width).height)
                .fold(0., f64::max);
            if let Some(constraint) = self.ivars().height.get()
                && (constraint.constant() - needed).abs() > 0.5
            {
                constraint.setConstant(needed);
            }
            let y = layout(&ty, &self.ivars().plan, width).action;
            for view in self.subviews().iter() {
                view.setFrame(NSRect::new(
                    NSPoint::new(p, y),
                    NSSize::new(width, action_height()),
                ));
            }
        }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            let names = unsafe { [NSAppearanceNameAqua, NSAppearanceNameDarkAqua] };
            let dark = self
                .effectiveAppearance()
                .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&names))
                .is_some_and(|name| &*name == names[1]);
            let card = self.backingAlignedRect_options(
                self.bounds(),
                NSAlignmentOptions::AlignAllEdgesNearest,
            );
            let pixel = 1. / self.window().map_or(2., |w| w.backingScaleFactor());
            let state = self.ivars();
            draw_card(&state.plan, card, state.radius, dark, pixel);
        }
    }
);

fn gradient() -> objc2::rc::Allocated<NSGradient> {
    <NSGradient as objc2::AnyThread>::alloc()
}
/// `text` from `at`, on one line cut with an ellipsis at `width`; its width.
fn line(text: &str, font: &NSFont, ink: &NSColor, at: NSPoint, width: f64) -> f64 {
    lines(text, font, ink, at, width, line_height(font))
}
/// `text` wrapped at `width` within `height`; its one-line width.
fn lines(text: &str, font: &NSFont, ink: &NSColor, at: NSPoint, width: f64, height: f64) -> f64 {
    let attrs = attributes(font, ink);
    let string = NSString::from_str(text);
    unsafe {
        string.drawWithRect_options_attributes_context(
            NSRect::new(at, NSSize::new(width.max(0.), height)),
            NSStringDrawingOptions::UsesLineFragmentOrigin
                | NSStringDrawingOptions::UsesFontLeading
                | NSStringDrawingOptions::TruncatesLastVisibleLine,
            Some(&attrs),
            None,
        )
    };
    unsafe { string.sizeWithAttributes(Some(&attrs)) }.width
}

/// The check mark (@pierre/icons' IconCheck, Apache-2.0, as a stroke) in a
/// `side` square at `at`.
fn check(at: NSPoint, side: f64, ink: &NSColor) {
    let unit = side / 16.;
    let p = |x: f64, y: f64| NSPoint::new(at.x + x * unit, at.y + y * unit);
    let path = NSBezierPath::bezierPath();
    path.moveToPoint(p(2.75, 8.7));
    path.lineToPoint(p(6.25, 12.2));
    path.lineToPoint(p(13.3, 3.75));
    path.setLineWidth(1.5 * unit);
    path.setLineCapStyle(NSLineCapStyle::Round);
    path.setLineJoinStyle(NSLineJoinStyle::Round);
    ink.setStroke();
    path.stroke();
}

fn draw_card(plan: &Plan, card: NSRect, r: f64, dark: bool, pixel: f64) {
    let ty = Type::new();
    let here = Colors::of(if dark { Scheme::Dark } else { Scheme::Light });
    let shape = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(card, r, r);
    if plan.offered() {
        // Paper Powermove Aerolite: Night Zenith (#061322) with Sky-Blue
        // aurora glow (#0284C7 / #38BDF8) and crisp sky-blue rim.
        let night = srgb(pm_shell::brand::AEROLITE_NIGHT, 1.);
        let deep = srgb(pm_shell::brand::AEROLITE_DEEP, 1.);
        let sky = srgb(pm_shell::brand::AEROLITE_SKY, 1.);
        let stops = NSArray::from_retained_slice(&[deep.clone(), night.clone(), night.clone()]);
        let locations = [0., 0.46, 1.];
        // SAFETY: three locations for three colours, in the device's sRGB.
        if let Some(fade) = unsafe {
            NSGradient::initWithColors_atLocations_colorSpace(
                gradient(),
                &stops,
                locations.as_ptr(),
                &objc2_app_kit::NSColorSpace::sRGBColorSpace(),
            )
        } {
            fade.drawInBezierPath_angle(&shape, 90.);
        }
        if let Some(pool) = NSGradient::initWithStartingColor_endingColor(
            gradient(),
            &sky.colorWithAlphaComponent(0.24),
            &sky.colorWithAlphaComponent(0.),
        ) {
            objc2_app_kit::NSGraphicsContext::saveGraphicsState_class();
            shape.addClip();
            let centre = NSPoint::new(
                card.origin.x + card.size.width * 0.5,
                card.origin.y + card.size.height * 0.04,
            );
            pool.drawFromCenter_radius_toCenter_radius_options(
                centre,
                0.,
                centre,
                card.size.width * 0.72,
                objc2_app_kit::NSGradientDrawingOptions::empty(),
            );
            objc2_app_kit::NSGraphicsContext::restoreGraphicsState_class();
        }
        srgb(pm_shell::brand::AEROLITE_SKY, 0.28).setStroke();
        let edge = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
            NSRect::new(
                NSPoint::new(card.origin.x + 0.5, card.origin.y + 0.5),
                NSSize::new(card.size.width - 1.0, card.size.height - 1.0),
            ),
            r - 0.5,
            r - 0.5,
        );
        edge.setLineWidth(1.0);
        edge.stroke();
    } else {
        srgb(here.text, Ink::new().well as f64).setFill();
        shape.fill();
        NSColor::separatorColor().setStroke();
        shape.setLineWidth(0.5);
        shape.stroke();
    }
    let (white, light) = (plan.offered(), |a: f64| srgb(0xffffff, a));
    let label = if white {
        light(1.)
    } else {
        srgb(here.text, 1.)
    };
    let secondary = if white {
        srgb(pm_shell::brand::AEROLITE_TINT, 0.78)
    } else {
        srgb(here.secondary, 1.)
    };
    let quiet = if white {
        srgb(pm_shell::brand::AEROLITE_TINT, 0.60)
    } else {
        srgb(here.muted, 1.)
    };
    let p = padding();
    let (x, width) = (card.origin.x + p, card.size.width - 2. * p);
    let at = layout(&ty, plan, width);
    let top = card.origin.y;
    if white
        && let Some((wm, wm_w)) = pm_shell::brand::aerolite_wordmark_ns(20.)
    {
        let fit_w = wm_w.min(width);
        let fit_h = 20. * (fit_w / wm_w.max(1.));
        wm.drawInRect(NSRect::new(
            NSPoint::new(x, top + p - top_trim(&ty) - (fit_h - line_height(&ty.name)) * 0.45),
            NSSize::new(fit_w, fit_h),
        ));
    } else {
        line(
            &plan.name,
            &ty.name,
            &label,
            NSPoint::new(x, top + p - top_trim(&ty)),
            width,
        );
    }
    let price_width = line(
        &plan.price,
        &ty.price,
        &label,
        NSPoint::new(x, top + at.price),
        width,
    );
    let gap = space().xs as f64;
    let baseline = ty.price.ascender() - ty.period.ascender();
    line(
        &plan.period,
        &ty.period,
        &quiet,
        NSPoint::new(x + price_width + gap, top + at.price + baseline),
        width - price_width - gap,
    );
    lines(
        &plan.note,
        &ty.note,
        &secondary,
        NSPoint::new(x, top + at.note),
        width,
        wrapped(&plan.note, &ty.note, width).max(line_height(&ty.note)),
    );
    if let Action::Status(status) = &plan.action {
        let y = top + at.action + (action_height() - line_height(&ty.note)) / 2.;
        line(status, &ty.note, &quiet, NSPoint::new(x, y), width);
    }
    if white {
        srgb(pm_shell::brand::AEROLITE_LIGHT, 0.16)
    } else {
        NSColor::separatorColor()
    }
    .setFill();
    NSBezierPath::fillRect(NSRect::new(
        NSPoint::new(x, top + at.hairline),
        NSSize::new(width, pixel),
    ));
    let side = line_height(&ty.feature);
    let text_x = x + side + space().sm as f64;
    let text = width - (text_x - x);
    let mark = if white {
        srgb(pm_shell::brand::AEROLITE_SKY, 1.)
    } else {
        secondary.clone()
    };
    for ((title, detail), (y, below)) in plan.features.iter().zip(&at.features) {
        check(NSPoint::new(x, top + y), side, &mark);
        line(
            title,
            &ty.feature,
            &label,
            NSPoint::new(text_x, top + y),
            text,
        );
        let h = wrapped(detail, &ty.detail, text);
        lines(
            detail,
            &ty.detail,
            &quiet,
            NSPoint::new(text_x, top + below),
            text,
            h,
        );
    }
}

/// The action's target: a button's tag is its plan's index.
pub(super) struct Target(async_channel::Sender<Value>);
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsPlanTarget"]
    #[ivars = Target]
    pub(super) struct PlanTarget;
    unsafe impl NSObjectProtocol for PlanTarget {}
    impl PlanTarget {
        #[unsafe(method(choose:))]
        fn choose(&self, sender: &NSButton) {
            let _ = self.ivars().0.try_send(json!(sender.tag()));
        }
    }
);

/// The plans side by side, each card the same width and height, `space.lg`
/// apart.
pub(super) struct Plans {
    pub(super) stack: Retained<NSStackView>,
    buttons: Vec<Retained<NSButton>>,
    _target: Retained<PlanTarget>,
}

impl Plans {
    pub(super) fn new(
        mtm: objc2::MainThreadMarker,
        label: &str,
        options: Vec<String>,
        radius: f64,
        send: async_channel::Sender<Value>,
    ) -> Self {
        let stack = NSStackView::new(mtm);
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        stack.setDistribution(NSStackViewDistribution::FillEqually);
        stack.setSpacing(space().lg as f64);
        // Top separates from the Plans subtitle; bottom + row_padding (6 + 6 = 12)
        // matches row_inset (12) on the left and right edges of the group card.
        stack.setEdgeInsets(NSEdgeInsets {
            top: space().md as f64,
            left: 0.,
            bottom: space().sm as f64,
            right: 0.,
        });
        stack.setTranslatesAutoresizingMaskIntoConstraints(false);
        stack.setAccessibilityRole(Some(unsafe { NSAccessibilityGroupRole }));
        stack.setAccessibilityLabel(Some(&NSString::from_str(label)));
        let target: Retained<PlanTarget> =
            unsafe { msg_send![super(PlanTarget::alloc(mtm).set_ivars(Target(send))), init] };
        let plans: Vec<Plan> = options.iter().map(|o| Plan::parse(o)).collect();
        let all_plans = Rc::new(plans.clone());
        // Every card as tall as the tallest, starting at the column's width
        // and updated on `layout()` when the real laid-out width is known.
        let ty = Type::new();
        let width = card_width() - 2. * padding();
        let height = plans
            .iter()
            .map(|p| layout(&ty, p, width).height)
            .fold(0., f64::max);
        let mut buttons = vec![];
        for (index, plan) in plans.into_iter().enumerate() {
            let spoken = plan.spoken();
            let button_title = match &plan.action {
                Action::Button(title) => Some(title.clone()),
                _ => None,
            };
            let card: Retained<Card> = unsafe {
                msg_send![
                    super(Card::alloc(mtm).set_ivars(CardState {
                        plan,
                        all_plans: all_plans.clone(),
                        height: OnceCell::new(),
                        radius,
                    })),
                    init
                ]
            };
            card.setTranslatesAutoresizingMaskIntoConstraints(false);
            card.setClipsToBounds(false);
            let height_constraint = card.heightAnchor().constraintEqualToConstant(height);
            height_constraint.setActive(true);
            let _ = card.ivars().height.set(height_constraint);
            card.setAccessibilityElement(false);
            card.setAccessibilityLabel(Some(&NSString::from_str(&spoken)));
            if let Some(title) = button_title {
                // A real push button, Paper Powermove Aerolite Sky CTA pill
                // with high-contrast Night Zenith label, full width in the card.
                let button = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &NSString::from_str(&title),
                        Some(&target),
                        Some(objc2::sel!(choose:)),
                        mtm,
                    )
                };
                button.setBezelStyle(NSBezelStyle::Push);
                button.setControlSize(NSControlSize::Regular);
                button.setBezelColor(Some(&srgb(pm_shell::brand::AEROLITE_SKY, 1.)));
                let btn_attrs = attributes(
                    &font(text_size().base, st().heading_weight),
                    &srgb(pm_shell::brand::AEROLITE_NIGHT, 1.),
                );
                let attr_title = unsafe {
                    NSAttributedString::initWithString_attributes(
                        NSAttributedString::alloc(),
                        &NSString::from_str(&title),
                        Some(&btn_attrs),
                    )
                };
                button.setAttributedTitle(&attr_title);
                button.setTag(index as isize);
                // The night card's appearance: untinted (its window not
                // key) it is dark glass with light text, never lost.
                button.setAppearance(
                    objc2_app_kit::NSAppearance::appearanceNamed(unsafe {
                        NSAppearanceNameDarkAqua
                    })
                    .as_deref(),
                );
                card.addSubview(&button);
                buttons.push(button);
            }
            stack.addArrangedSubview(&card);
        }
        Self {
            stack,
            buttons,
            _target: target,
        }
    }
    /// Busy (Checkout open, or the account changing): the buttons wait.
    pub(super) fn update(&self, disabled: bool) {
        for button in &self.buttons {
            button.setEnabled(!disabled);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Plan};

    #[test]
    fn a_plan_reads_its_lines_in_order() {
        let plan = Plan::parse(
            "Aerolite\n$9.99\nper month\nYour editing agent, with inference included.\nGet Aerolite\n1,000 credits a month\tInference for Aerolite, run by Powermove",
        );
        assert_eq!(plan.action, Action::Button("Get Aerolite".into()));
        assert!(plan.offered());
        assert_eq!(
            plan.features,
            [(
                "1,000 credits a month".to_owned(),
                "Inference for Aerolite, run by Powermove".to_owned()
            )]
        );
        let free = Plan::parse(
            "Free\n$0\n\nThe whole editor, with your own agent.\n=Current plan\nThe whole editor\tEvery tool, effect and export",
        );
        assert_eq!(free.action, Action::Status("Current plan".into()));
        assert!(!free.offered());
        assert_eq!(
            free.spoken(),
            "Free, $0, The whole editor, with your own agent., Current plan, The whole editor, Every tool, effect and export"
        );
    }
}
