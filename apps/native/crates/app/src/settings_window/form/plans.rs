//! Settings › Account's plan picker on Free: one card per plan in the
//! Settings card anatomy (its fill, hairline edge and nested corner), its
//! lines the plan's name, price, monthly credits and, for the better value,
//! how much more it gives per dollar. The chosen card is ringed in the
//! accent, the gap and ring the other Settings pickers use, concentric with
//! the card. No art. Every size and colour is a token.
use super::{font, regular, space, st, text_size};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityRadioButtonRole, NSAccessibilityRadioGroupRole,
    NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBezierPath,
    NSColor, NSEvent, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSStackView,
    NSStackViewDistribution, NSStringDrawing, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{NSArray, NSDictionary, NSEdgeInsets, NSPoint, NSRect, NSSize, NSString};
use pm_design::{Colors, Ink, Scheme};
use serde_json::{Value, json};
use std::cell::Cell;

fn colour(rgb: u32, alpha: f64) -> Retained<NSColor> {
    let channel = |shift: u32| ((rgb >> shift) & 255) as f64 / 255.;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), alpha)
}
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
/// Inside a card: the row's side inset all round.
fn padding() -> f64 {
    st().row_inset as f64
}
/// The name's font, then every other line's.
fn fonts() -> (Retained<NSFont>, Retained<NSFont>) {
    (
        font(text_size().base, st().heading_weight),
        font(text_size().sm, regular()),
    )
}
fn line_height(font: &NSFont) -> f64 {
    let attrs = attributes(font, &NSColor::labelColor());
    unsafe { NSString::from_str("Ag").sizeWithAttributes(Some(&attrs)) }
        .height
        .ceil()
}

pub(super) struct CardState {
    lines: Vec<String>,
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
            let s = st();
            // The card is the view; its ring stands outside it (the view
            // doesn't clip), so cards line up with the rows' text.
            let card = self.bounds();
            let r = state.radius;
            // A well a step off the group card, with the cards' hairline edge.
            colour(c.text, Ink::new().well as f64).setFill();
            let shape = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(card, r, r);
            shape.fill();
            NSColor::separatorColor().setStroke();
            shape.setLineWidth(0.5);
            shape.stroke();
            if state.selected.get() {
                // Concentric: the card's corner plus the gap to the ring's
                // centre line.
                let o = s.picker_ring_gap as f64 + s.picker_ring as f64 / 2.;
                let ring = NSRect::new(
                    NSPoint::new(card.origin.x - o, card.origin.y - o),
                    NSSize::new(card.size.width + 2. * o, card.size.height + 2. * o),
                );
                let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    ring,
                    r + o,
                    r + o,
                );
                colour(c.accent, 1.).setStroke();
                path.setLineWidth(s.picker_ring as f64);
                path.stroke();
            }
            let (name_font, line_font) = fonts();
            let alpha = if state.enabled.get() { 1. } else { 0.5 };
            let mut y = card.origin.y + padding();
            for (i, line) in state.lines.iter().enumerate() {
                let (font, ink) = if i == 0 {
                    (&name_font, colour(c.text, alpha))
                } else {
                    (&line_font, colour(c.secondary, alpha))
                };
                let attrs = attributes(font, &ink);
                let text = NSString::from_str(line);
                unsafe {
                    text.drawAtPoint_withAttributes(
                        NSPoint::new(card.origin.x + padding(), y),
                        Some(&attrs),
                    )
                };
                y += line_height(font) + if i == 0 { space().xs as f64 } else { 0. };
            }
        }
    }
);

impl Card {
    fn choose(&self) {
        let state = self.ivars();
        if state.enabled.get() {
            let _ = state.send.try_send(json!(state.index));
        }
    }
}

/// The plans side by side, each card the same width, `space.lg` apart.
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
        // The chosen card's ring stands outside it: room for it above.
        let ring = (st().picker_ring_gap + st().picker_ring) as f64;
        stack.setEdgeInsets(NSEdgeInsets {
            top: space().sm as f64 + ring,
            left: 0.,
            bottom: space().hair as f64 + ring,
            right: 0.,
        });
        stack.setTranslatesAutoresizingMaskIntoConstraints(false);
        stack.setAccessibilityRole(Some(unsafe { NSAccessibilityRadioGroupRole }));
        stack.setAccessibilityLabel(Some(&NSString::from_str(label)));
        // Every card as tall as the one with the most lines.
        let (name_font, line_font) = fonts();
        let most = options.iter().map(|o| o.lines().count()).max().unwrap_or(1);
        let height = 2. * padding()
            + line_height(&name_font)
            + space().xs as f64
            + (most.saturating_sub(1)) as f64 * line_height(&line_font);
        let cards = options
            .into_iter()
            .enumerate()
            .map(|(index, option)| {
                let lines: Vec<String> = option.lines().map(str::to_owned).collect();
                let card: Retained<Card> = unsafe {
                    msg_send![
                        super(Card::alloc(mtm).set_ivars(CardState {
                            lines: lines.clone(),
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
                card.setAccessibilityLabel(Some(&NSString::from_str(&lines.join(", "))));
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
