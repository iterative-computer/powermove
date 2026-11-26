//! Settings › Account's plan picker on Free: one card per plan, laid out as
//! the website's plan cards in the Settings card anatomy (its well, hairline
//! edge and nested corner). Top, from the top edge: the plan's name, its
//! price large with a quiet period after it, one line saying who it is for.
//! Bottom, from the bottom edge: a hairline, then the features, each a check
//! mark and a line (and a quieter detail under it, when it has one). The padding is the same on
//! all four sides; the room a taller neighbour leaves falls between the two
//! halves, never under the last line. The chosen card wears the shared
//! selection ring (`ring::Ring`). No art; every size and colour is a token.
//!
//! An option's label is its lines, in order: name, price, period, note,
//! then one line per feature (`title\tdetail`).
use super::ring::{Ring, srgb};
use super::{font, regular, space, st, text_size};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityRadioButtonRole, NSAccessibilityRadioGroupRole,
    NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBezierPath,
    NSColor, NSEvent, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSLineCapStyle,
    NSLineJoinStyle, NSStackView, NSStackViewDistribution, NSStringDrawing, NSStringDrawingOptions,
    NSStringNSExtendedStringDrawing, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{
    NSAlignmentOptions, NSArray, NSDictionary, NSEdgeInsets, NSPoint, NSRect, NSSize, NSString,
};
use pm_design::{Colors, Ink, Scheme};
use serde_json::{Value, json};
use std::cell::Cell;

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
/// Inside a card, the same on every side.
fn padding() -> f64 {
    space().xl as f64
}

/// One plan, read from its option's lines.
#[derive(Clone, Debug, Default, PartialEq)]
struct Plan {
    name: String,
    price: String,
    period: String,
    note: String,
    features: Vec<(String, String)>,
}
impl Plan {
    fn parse(option: &str) -> Self {
        let mut lines = option.lines();
        let mut next = || lines.next().unwrap_or_default().to_owned();
        let (name, price, period, note) = (next(), next(), next(), next());
        let features = option
            .lines()
            .skip(4)
            .map(|line| {
                let (title, detail) = line.split_once('\t').unwrap_or((line, ""));
                (title.to_owned(), detail.to_owned())
            })
            .collect();
        Self {
            name,
            price,
            period,
            note,
            features,
        }
    }
    /// What VoiceOver reads: every line, in order.
    fn spoken(&self) -> String {
        let mut parts = vec![
            self.name.clone(),
            format!(
                "{} per {}",
                self.price,
                self.period.trim_start_matches('/').trim()
            ),
            self.note.clone(),
        ];
        for (title, detail) in &self.features {
            parts.push(title.clone());
            parts.push(detail.clone());
        }
        parts.retain(|p| !p.trim().is_empty());
        parts.join(", ")
    }
}

/// The card's type: name, price, period, note, feature, feature detail.
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
        Self {
            name: font(t.base, s.heading_weight),
            price: font(t.x4l, s.heading_weight),
            period: font(t.sm, regular()),
            note: font(t.sm, regular()),
            feature: font(t.sm, s.heading_weight),
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
/// The check mark's square: the feature line's height.
fn mark(ty: &Type) -> f64 {
    line_height(&ty.feature)
}
/// The note wraps to at most this many lines.
const NOTE_LINES: f64 = 2.;
/// The top half: name, price, note.
fn top_height(ty: &Type) -> f64 {
    line_height(&ty.name)
        + space().sm as f64
        + line_height(&ty.price)
        + space().xs as f64
        + NOTE_LINES * line_height(&ty.note)
}
/// One feature: its line, and its detail under it when it has one.
fn feature_height(ty: &Type, detail: &str) -> f64 {
    line_height(&ty.feature)
        + if detail.is_empty() {
            0.
        } else {
            space().hair as f64 / 2. + line_height(&ty.detail)
        }
}
/// The features, `space.sm` apart.
fn features_height(ty: &Type, features: &[(String, String)]) -> f64 {
    features
        .iter()
        .map(|(_, d)| feature_height(ty, d))
        .sum::<f64>()
        + features.len().saturating_sub(1) as f64 * space().sm as f64
}
/// A card's least height: the padding, the top half, the hairline
/// `space.lg` from each side, the features.
fn card_height(ty: &Type, features: &[(String, String)]) -> f64 {
    (2. * padding() + top_height(ty) + 2. * space().lg as f64 + features_height(ty, features)
        - top_trim(ty)
        - bottom_trim(ty, features))
    .ceil()
}
/// The padding is measured to the ink, as `text-box: trim-both cap
/// alphabetic` does: from the top edge to the name's cap height, from the
/// bottom edge to the last line's baseline. What the name's line box holds
/// above its caps:
fn top_trim(ty: &Type) -> f64 {
    ty.name.ascender() - ty.name.capHeight()
}
/// What the last line's box holds under its baseline.
fn bottom_trim(ty: &Type, features: &[(String, String)]) -> f64 {
    let font = match features.last() {
        Some((_, detail)) if !detail.is_empty() => &ty.detail,
        Some(_) => &ty.feature,
        None => return 0.,
    };
    line_height(font) - font.ascender()
}

pub(super) struct CardState {
    plan: Plan,
    index: usize,
    selected: Cell<bool>,
    enabled: Cell<bool>,
    radius: f64,
    send: async_channel::Sender<Value>,
}

define_class!(
    /// One plan, drawn as a Settings card; a click chooses it.
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
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _: Option<&NSEvent>) -> bool {
            true
        }
        #[unsafe(method(mouseDown:))]
        fn down(&self, _: &NSEvent) {
            self.choose();
        }
        #[unsafe(method(accessibilityPerformPress))]
        fn press(&self) -> bool {
            self.choose();
            true
        }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            let state = self.ivars();
            let names = unsafe { [NSAppearanceNameAqua, NSAppearanceNameDarkAqua] };
            let dark = self
                .effectiveAppearance()
                .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&names))
                .is_some_and(|name| &*name == names[1]);
            let c = Colors::of(if dark { Scheme::Dark } else { Scheme::Light });
            // The card is the view, on whole device pixels; the ring stands
            // outside it (the view doesn't clip), so the cards' edges line
            // up with the rows' text.
            let card = self.backingAlignedRect_options(
                self.bounds(),
                NSAlignmentOptions::AlignAllEdgesNearest,
            );
            let r = state.radius;
            let shape = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(card, r, r);
            srgb(c.text, Ink::new().well as f64).setFill();
            shape.fill();
            if state.selected.get() {
                let radii = pm_shell::scale().radius;
                Ring::of(&st()).stroke(&radii, card, r, &srgb(c.accent, 1.));
            } else {
                // The cards' hairline edge, inside the card.
                let edge = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    inset(card, 0.25),
                    r - 0.25,
                    r - 0.25,
                );
                NSColor::separatorColor().setStroke();
                edge.setLineWidth(0.5);
                edge.stroke();
            }
            let alpha = if state.enabled.get() { 1. } else { 0.5 };
            let pixel = 1. / self.window().map_or(2., |w| w.backingScaleFactor());
            draw_plan(&state.plan, card, &c, alpha, state.selected.get(), pixel);
        }
    }
);

fn inset(rect: NSRect, by: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(rect.origin.x + by, rect.origin.y + by),
        NSSize::new(rect.size.width - 2. * by, rect.size.height - 2. * by),
    )
}

/// `text` in `font` and `ink` from `at`, on one line cut with an ellipsis
/// at `width`; its width.
fn line(text: &str, font: &NSFont, ink: &NSColor, at: NSPoint, width: f64) -> f64 {
    lines(text, font, ink, at, width, 1.)
}
/// `text` wrapped at `width` to at most `count` lines, the last cut with an
/// ellipsis; its one-line width.
fn lines(text: &str, font: &NSFont, ink: &NSColor, at: NSPoint, width: f64, count: f64) -> f64 {
    let attrs = attributes(font, ink);
    let string = NSString::from_str(text);
    let height = count * line_height(font);
    unsafe {
        string.drawWithRect_options_attributes_context(
            NSRect::new(at, NSSize::new(width.max(0.), height)),
            NSStringDrawingOptions::UsesLineFragmentOrigin
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

/// `hairline`: one device pixel, in points.
fn draw_plan(plan: &Plan, card: NSRect, c: &Colors, alpha: f64, selected: bool, hairline: f64) {
    let ty = Type::new();
    let p = padding();
    let (x, width) = (card.origin.x + p, card.size.width - 2. * p);
    let label = srgb(c.text, alpha);
    let secondary = srgb(c.secondary, alpha);
    let quiet = srgb(c.muted, alpha);
    // Top: the name, the price with its period on the same baseline, the note.
    let mut y = card.origin.y + p - top_trim(&ty);
    line(&plan.name, &ty.name, &label, NSPoint::new(x, y), width);
    y += line_height(&ty.name) + space().sm as f64;
    let price_width = line(&plan.price, &ty.price, &label, NSPoint::new(x, y), width);
    let gap = space().xs as f64;
    let baseline = ty.price.ascender() - ty.period.ascender();
    line(
        &plan.period,
        &ty.period,
        &quiet,
        NSPoint::new(x + price_width + gap, y + baseline),
        width - price_width - gap,
    );
    y += line_height(&ty.price) + space().xs as f64;
    lines(
        &plan.note,
        &ty.note,
        &secondary,
        NSPoint::new(x, y),
        width,
        NOTE_LINES,
    );
    // Bottom, from the bottom edge up: the features under a hairline.
    let bottom = card.origin.y + card.size.height - p + bottom_trim(&ty, &plan.features);
    let mut y = bottom - features_height(&ty, &plan.features);
    NSColor::separatorColor().setFill();
    NSBezierPath::fillRect(NSRect::new(
        NSPoint::new(x, (y - space().lg as f64).round()),
        NSSize::new(width, hairline),
    ));
    let side = mark(&ty);
    let text_x = x + side + space().sm as f64;
    let check_ink = if selected {
        srgb(c.accent, alpha)
    } else {
        srgb(c.secondary, alpha)
    };
    for (title, detail) in &plan.features {
        check(NSPoint::new(x, y), side, &check_ink);
        line(
            title,
            &ty.feature,
            &label,
            NSPoint::new(text_x, y),
            width - (text_x - x),
        );
        if !detail.is_empty() {
            let below = y + line_height(&ty.feature) + space().hair as f64 / 2.;
            line(
                detail,
                &ty.detail,
                &secondary,
                NSPoint::new(text_x, below),
                width - (text_x - x),
            );
        }
        y += feature_height(&ty, detail) + space().sm as f64;
    }
}

impl Card {
    fn choose(&self) {
        let state = self.ivars();
        if state.enabled.get() {
            let _ = state.send.try_send(json!(state.index));
        }
    }
}

/// The plans side by side, each card the same width and height, `space.lg`
/// apart.
pub(super) struct Plans {
    pub(super) stack: Retained<NSStackView>,
    cards: Vec<Retained<Card>>,
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
        // The chosen card's ring stands outside it: its room above and below.
        let ring = Ring::of(&st()).margin();
        stack.setEdgeInsets(NSEdgeInsets {
            top: space().sm as f64 + ring,
            left: 0.,
            bottom: space().hair as f64 + ring,
            right: 0.,
        });
        stack.setTranslatesAutoresizingMaskIntoConstraints(false);
        stack.setAccessibilityRole(Some(unsafe { NSAccessibilityRadioGroupRole }));
        stack.setAccessibilityLabel(Some(&NSString::from_str(label)));
        let plans: Vec<Plan> = options.iter().map(|o| Plan::parse(o)).collect();
        // Every card as tall as the one whose features take the most room.
        let ty = Type::new();
        let height = plans
            .iter()
            .map(|p| card_height(&ty, &p.features))
            .fold(0., f64::max);
        let cards = plans
            .into_iter()
            .enumerate()
            .map(|(index, plan)| {
                let spoken = plan.spoken();
                let card: Retained<Card> = unsafe {
                    msg_send![
                        super(Card::alloc(mtm).set_ivars(CardState {
                            plan,
                            index,
                            selected: Cell::new(false),
                            enabled: Cell::new(true),
                            radius,
                            send: send.clone(),
                        })),
                        init
                    ]
                };
                card.setTranslatesAutoresizingMaskIntoConstraints(false);
                card.setClipsToBounds(false);
                card.heightAnchor()
                    .constraintEqualToConstant(height)
                    .setActive(true);
                card.setAccessibilityElement(true);
                card.setAccessibilityRole(Some(unsafe { NSAccessibilityRadioButtonRole }));
                card.setAccessibilityLabel(Some(&NSString::from_str(&spoken)));
                stack.addArrangedSubview(&card);
                card
            })
            .collect();
        Self { stack, cards }
    }
    pub(super) fn update(&self, selected: Option<usize>, disabled: bool) {
        for (i, card) in self.cards.iter().enumerate() {
            let state = card.ivars();
            let on = selected == Some(i);
            if state.selected.replace(on) != on || state.enabled.replace(!disabled) == disabled {
                card.setNeedsDisplay(true);
            }
            unsafe { card.setAccessibilityValue(Some(&objc2_foundation::NSNumber::new_bool(on))) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Plan;

    #[test]
    fn a_plan_reads_its_lines_in_order() {
        let plan = Plan::parse(
            "Aero\n$19.99\n/ month\nFor editors who lean on Aero every day.\n2,500 credits a month\t25% more credits per dollar",
        );
        assert_eq!(plan.name, "Aero");
        assert_eq!(plan.price, "$19.99");
        assert_eq!(plan.period, "/ month");
        assert_eq!(
            plan.features,
            [(
                "2,500 credits a month".to_owned(),
                "25% more credits per dollar".to_owned()
            )]
        );
        assert_eq!(
            plan.spoken(),
            "Aero, $19.99 per month, For editors who lean on Aero every day., 2,500 credits a month, 25% more credits per dollar"
        );
    }
}
