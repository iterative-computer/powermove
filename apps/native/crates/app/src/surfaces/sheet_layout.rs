//! Shared README §2 sheet geometry. N3: every dimension is an Opus token;
//! N7/N8: native controls publish through the caller's ID-addressed input target.
use pm_design::Tokens;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetLayout {
    pub width: f64,
    pub inset: f64,
    pub value_column: f64,
    pub row: f64,
    pub field: f64,
}
impl SheetLayout {
    pub fn resolve(t: &Tokens, wide: bool) -> Self {
        Self {
            width: f64::from(if wide {
                t.metrics.sheet_width_wide
            } else {
                t.metrics.sheet_width
            }),
            inset: t.metrics.sheet_inset.into(),
            value_column: t.metrics.sheet_value_column.into(),
            row: t.metrics.row.into(),
            field: t.metrics.field.into(),
        }
    }
    pub fn value_x(self) -> f64 {
        self.width - self.inset - self.value_column
    }
    pub fn label_width(self) -> f64 {
        self.value_x() - self.inset * 2.
    }
    /// Units sit in their own column after number fields, so every number's
    /// right edge lines up whatever its unit.
    pub fn unit_width(self) -> f64 {
        self.field
    }
    /// Where number fields end: before the unit column.
    pub fn number_right(self, gap: f64) -> f64 {
        self.width - self.inset - self.unit_width() - gap
    }
    /// A number field's width: two of them, a `×` and their gaps fill the
    /// value column up to the units (`1920 × 1080`).
    pub fn number_width(self, gap: f64) -> f64 {
        ((self.value_column - self.unit_width() - 3. * gap - self.cross_width()) / 2.).floor()
    }
    pub fn cross_width(self) -> f64 {
        (self.field / 2.).round()
    }
    /// The band under the last row: the footer's buttons, and the gap above
    /// them that holds the footer hairline.
    pub fn footer_band(self) -> f64 {
        self.row + self.inset
    }
}
#[cfg(target_os = "macos")]
pub mod native {
    use super::*;
    use objc2::{
        MainThreadMarker, MainThreadOnly,
        rc::Retained,
        runtime::{AnyObject, Sel},
    };
    use objc2_app_kit::*;
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::collections::BTreeMap;
    /// Real AppKit sheet, retained with its controls until the owner closes it.
    /// Control keys are stable field/action IDs, never translated labels.
    pub struct Sheet {
        pub window: Retained<NSWindow>,
        pub content: Retained<NSView>,
        pub fields: BTreeMap<String, Retained<NSTextField>>,
        pub buttons: BTreeMap<String, Retained<NSButton>>,
        pub controls: BTreeMap<String, Retained<NSControl>>,
        pub layout: SheetLayout,
        y: f64,
        mtm: MainThreadMarker,
        tokens: Tokens,
    }
    pub fn colour(rgb: u32) -> Retained<NSColor> {
        NSColor::colorWithRed_green_blue_alpha(
            f64::from((rgb >> 16) & 255) / 255.,
            f64::from((rgb >> 8) & 255) / 255.,
            f64::from(rgb & 255) / 255.,
            1.,
        )
    }
    impl Sheet {
        pub fn new(
            title: &str,
            rows: usize,
            tokens: &Tokens,
            wide: bool,
            mtm: MainThreadMarker,
        ) -> Self {
            let layout = SheetLayout::resolve(tokens, wide);
            // Each title, detail and help/error line occupies a token row. The
            // caller counts its real lines; no fixed guessed sheet height.
            let height = layout.inset * 2. + layout.row * (rows + 1) as f64 + layout.footer_band();
            let rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(layout.width, height));
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    rect,
                    NSWindowStyleMask::Titled,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            window.setTitle(&NSString::from_str(title));
            let content = NSView::initWithFrame(NSView::alloc(mtm), rect);
            window.setContentView(Some(&content));
            let mut sheet = Self {
                window,
                content,
                fields: BTreeMap::new(),
                buttons: BTreeMap::new(),
                controls: BTreeMap::new(),
                layout,
                y: height - layout.inset,
                mtm,
                tokens: tokens.clone(),
            };
            sheet.line(title, tokens.typography.heading.into(), tokens.colors.text);
            sheet
        }
        pub fn line(&mut self, text: &str, size: f64, rgb: u32) -> Retained<NSTextField> {
            self.line_reserving(text, text, size, rgb)
        }
        /// A line tall enough for `reserve` too, for text that changes while
        /// the sheet is open.
        pub fn line_reserving(
            &mut self,
            text: &str,
            reserve: &str,
            size: f64,
            rgb: u32,
        ) -> Retained<NSTextField> {
            let label =
                NSTextField::wrappingLabelWithString(&NSString::from_str(reserve), self.mtm);
            label.setFont(Some(&NSFont::systemFontOfSize(size)));
            label.setTextColor(Some(&colour(rgb)));
            let width = self.layout.width - 2. * self.layout.inset;
            label.setPreferredMaxLayoutWidth(width);
            let fit = |label: &NSTextField| label.sizeThatFits(NSSize::new(width, f64::MAX)).height;
            let reserved = fit(&label);
            label.setStringValue(&NSString::from_str(text));
            let height = fit(&label).max(reserved).ceil().max(self.layout.row);
            let extra = height - self.layout.row;
            if extra > 0. {
                let mut size = self.content.frame().size;
                size.height += extra;
                self.window.setContentSize(size);
                self.content.setFrameSize(size);
                for child in self.content.subviews() {
                    let mut origin = child.frame().origin;
                    origin.y += extra;
                    child.setFrameOrigin(origin);
                }
                self.y += extra;
            }
            self.y -= height;
            label.setFrame(NSRect::new(
                NSPoint::new(self.layout.inset, self.y),
                NSSize::new(width, height),
            ));
            self.content.addSubview(&label);
            label
        }
        pub fn row(&mut self, label: &str, view: &NSView) {
            self.label_row(label);
            self.place(view, self.layout.value_x(), self.layout.value_column);
        }
        /// Starts a row: its label on the left. Controls go at `self.y`.
        fn label_row(&mut self, label: &str) {
            self.y -= self.layout.row;
            let field = NSTextField::labelWithString(&NSString::from_str(label), self.mtm);
            field.setFont(Some(&NSFont::systemFontOfSize(
                self.tokens.typography.body.into(),
            )));
            field.setTextColor(Some(&colour(self.tokens.colors.secondary)));
            self.place_label(&field, self.layout.inset, self.layout.label_width());
        }
        /// A label centred on the row's field height, so its text sits level
        /// with the control beside it.
        fn place_label(&self, label: &NSTextField, x: f64, width: f64) {
            let height = label
                .sizeThatFits(NSSize::new(width, f64::MAX))
                .height
                .ceil();
            let y = self.y + ((self.layout.field - height) / 2.).round();
            label.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
            self.content.addSubview(label);
        }
        fn place(&self, view: &NSView, x: f64, width: f64) {
            view.setFrame(NSRect::new(
                NSPoint::new(x, self.y),
                NSSize::new(width, self.layout.field),
            ));
            self.content.addSubview(view);
        }
        fn gap(&self) -> f64 {
            self.tokens.metrics.gap.into()
        }
        /// A right-aligned number field, its id and AX label set.
        fn number_field(
            &self,
            id: &str,
            label: &str,
            value: &str,
            enabled: bool,
        ) -> Retained<NSTextField> {
            let field = NSTextField::textFieldWithString(&NSString::from_str(value), self.mtm);
            field.setFocusRingType(NSFocusRingType::None);
            field.setAlignment(NSTextAlignment::Right);
            field.setIdentifier(Some(&NSString::from_str(id)));
            field.setAccessibilityLabel(Some(&NSString::from_str(label)));
            field.setEnabled(enabled);
            field
        }
        fn small_label(&self, text: &str) -> Retained<NSTextField> {
            let label = NSTextField::labelWithString(&NSString::from_str(text), self.mtm);
            label.setFont(Some(&NSFont::systemFontOfSize(
                self.tokens.typography.minimum.into(),
            )));
            label.setTextColor(Some(&colour(self.tokens.colors.muted)));
            label
        }
        /// The unit column after the number fields: a unit, or a small control
        /// such as the size's aspect link.
        pub fn trailing(&self, view: &NSView) {
            let x = self.layout.number_right(self.gap()) + self.gap();
            self.place(view, x, self.layout.unit_width());
        }
        fn unit(&self, text: &str) {
            let x = self.layout.number_right(self.gap()) + self.gap();
            self.place_label(&self.small_label(text), x, self.layout.unit_width());
        }
        pub fn text(&mut self, id: &str, label: &str, value: &str, secure: bool, enabled: bool) {
            let field: Retained<NSTextField> = if secure {
                let field = NSSecureTextField::initWithFrame(
                    NSSecureTextField::alloc(self.mtm),
                    NSRect::ZERO,
                );
                field.into_super()
            } else {
                NSTextField::textFieldWithString(&NSString::from_str(""), self.mtm)
            };
            field.setStringValue(&NSString::from_str(value));
            // Owner rule: no accent focus rings; the bezel and caret show focus.
            field.setFocusRingType(NSFocusRingType::None);
            field.setIdentifier(Some(&NSString::from_str(id)));
            field.setAccessibilityLabel(Some(&NSString::from_str(label)));
            field.setEnabled(enabled);
            self.row(label, &field);
            self.fields.insert(id.into(), field);
        }
        pub fn number(&mut self, id: &str, label: &str, value: &str, unit: &str, enabled: bool) {
            self.label_row(label);
            let (gap, width) = (self.gap(), self.layout.number_width(self.gap()));
            let field = self.number_field(id, label, value, enabled);
            self.place(&field, self.layout.number_right(gap) - width, width);
            if !unit.is_empty() {
                self.unit(unit);
            }
            self.fields.insert(id.into(), field);
        }
        /// Two numbers on one row, `a × b` (a size), each its own field and id.
        pub fn pair(
            &mut self,
            label: &str,
            [(a, a_label, a_value), (b, b_label, b_value)]: [(&str, &str, &str); 2],
            unit: &str,
            enabled: bool,
        ) {
            self.label_row(label);
            let (gap, width) = (self.gap(), self.layout.number_width(self.gap()));
            let right = self.layout.number_right(gap);
            let second = self.number_field(b, b_label, b_value, enabled);
            self.place(&second, right - width, width);
            let cross = self.small_label("×");
            cross.setAlignment(NSTextAlignment::Center);
            let cross_x = right - width - gap - self.layout.cross_width();
            self.place_label(&cross, cross_x, self.layout.cross_width());
            let first = self.number_field(a, a_label, a_value, enabled);
            self.place(&first, cross_x - gap - width, width);
            if !unit.is_empty() {
                self.unit(unit);
            }
            self.fields.insert(a.into(), first);
            self.fields.insert(b.into(), second);
        }
        /// A colour well as a swatch, not a value column-wide bar.
        pub fn swatch(&mut self, id: &str, label: &str, well: Retained<NSColorWell>) {
            well.setColorWellStyle(NSColorWellStyle::Minimal);
            well.setAccessibilityLabel(Some(&NSString::from_str(label)));
            well.setIdentifier(Some(&NSString::from_str(id)));
            self.label_row(label);
            self.place(&well, self.layout.value_x(), 2. * self.layout.field);
            self.controls.insert(id.into(), well.into_super());
        }
        /// A radio or checkbox whose title is its own full-width label.
        pub fn option(&mut self, id: &str, control: Retained<NSControl>) {
            self.y -= self.layout.row;
            control.setIdentifier(Some(&NSString::from_str(id)));
            control.setFrame(NSRect::new(
                NSPoint::new(self.layout.inset, self.y),
                NSSize::new(
                    self.layout.width - 2. * self.layout.inset,
                    self.layout.field,
                ),
            ));
            self.content.addSubview(&control);
            self.controls.insert(id.into(), control);
        }
        pub fn control(&mut self, id: &str, label: &str, control: Retained<NSControl>) {
            control.setAccessibilityLabel(Some(&NSString::from_str(label)));
            control.setIdentifier(Some(&NSString::from_str(id)));
            self.row(label, &control);
            self.controls.insert(id.into(), control);
        }
        pub fn button(
            &mut self,
            id: &str,
            title: &str,
            enabled: bool,
            default: bool,
            cancel: bool,
        ) {
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(title),
                    None,
                    None,
                    self.mtm,
                )
            };
            button.setEnabled(enabled);
            button.setIdentifier(Some(&NSString::from_str(id)));
            button.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
            button.setKeyEquivalent(&NSString::from_str(if cancel {
                "\u{1b}"
            } else if default {
                "\r"
            } else {
                ""
            }));
            button.sizeToFit();
            if default {
                let cell = button
                    .cell()
                    .and_then(|c| c.downcast::<NSButtonCell>().ok());
                self.window.setDefaultButtonCell(cell.as_deref());
            }
            self.buttons.insert(id.into(), button);
        }
        /// Footer order is the spec's reading order, with a leading tertiary action.
        pub fn footer(&self, order: &[&str], leading: Option<&str>) {
            // v1's footer sits under a hairline across the sheet.
            let rule = NSBox::new(self.mtm);
            rule.setBoxType(NSBoxType::Separator);
            rule.setFrame(NSRect::new(
                NSPoint::new(
                    0.,
                    self.layout.inset + self.layout.row + self.layout.inset / 2.,
                ),
                NSSize::new(self.layout.width, 1.),
            ));
            self.content.addSubview(&rule);
            let mut x = self.layout.width - self.layout.inset;
            for id in order.iter().rev() {
                let b = &self.buttons[*id];
                let size = b.frame().size;
                x -= size.width;
                b.setFrameOrigin(NSPoint::new(x, self.layout.inset));
                self.content.addSubview(b);
                x -= f64::from(self.tokens.metrics.gap);
            }
            if let Some(id) = leading {
                let b = &self.buttons[id];
                b.setFrameOrigin(NSPoint::new(self.layout.inset, self.layout.inset));
                self.content.addSubview(b);
            }
        }
        /// Main-thread shell binds native targets that only enqueue typed IDs.
        /// # Safety
        /// The target must implement selector(control:) and outlive this sheet.
        pub unsafe fn bind(&self, target: &AnyObject, action: Sel) {
            for control in self
                .buttons
                .values()
                .map(|v| &**v as &NSControl)
                .chain(self.fields.values().map(|v| &**v as &NSControl))
                .chain(self.controls.values().map(|v| &**v))
            {
                unsafe {
                    control.setTarget(Some(target));
                    control.setAction(Some(action));
                }
            }
        }
        pub fn present(&self, owner: &NSWindow, first: Option<&str>) {
            owner.beginSheet_completionHandler(&self.window, None);
            if let Some(field) = first.and_then(|id| self.fields.get(id)) {
                self.window.makeFirstResponder(Some(field));
                unsafe { field.selectText(None) };
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_uses_host_tokens_in_every_appearance() {
        for scheme in [pm_design::Scheme::Light, pm_design::Scheme::Dark] {
            for increase_contrast in [false, true] {
                let mut t = Tokens::new(
                    scheme,
                    pm_design::Accessibility {
                        increase_contrast,
                        ..Default::default()
                    },
                );
                t.metrics.sheet_inset += 3.;
                t.metrics.sheet_value_column += 5.;
                for wide in [false, true] {
                    let g = SheetLayout::resolve(&t, wide);
                    assert_eq!(g.inset, f64::from(t.metrics.sheet_inset));
                    assert_eq!(g.value_column, f64::from(t.metrics.sheet_value_column));
                    assert_eq!(
                        g.width,
                        f64::from(if wide {
                            t.metrics.sheet_width_wide
                        } else {
                            t.metrics.sheet_width
                        })
                    );
                    assert_eq!(g.value_x() + g.value_column + g.inset, g.width);
                    assert!(g.label_width() > 0.);
                }
            }
        }
    }
}
