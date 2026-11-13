//! AppKit sheets in the Settings anatomy: a title, then cards of rows on one
//! text column. A row is its title over its notes, with a real control of its
//! natural width at the trailing edge. Every dimension is a design token;
//! controls publish through the caller's ID-addressed input target.
use pm_design::Tokens;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetLayout {
    pub width: f64,
    /// The sheet's margin: where its cards and its buttons end.
    pub inset: f64,
    /// A row's inset inside its card.
    pub pad: f64,
    /// The widest a trailing control stands.
    pub value_column: f64,
    /// The least height of a row that carries a control.
    pub row: f64,
    pub field: f64,
    pub gap: f64,
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
            pad: t.metrics.inset.into(),
            value_column: t.metrics.sheet_value_column.into(),
            row: t.metrics.form_row.into(),
            field: t.metrics.field.into(),
            gap: t.metrics.gap.into(),
        }
    }
    /// The one x every line of text starts on: the sheet's title, section
    /// headings, row titles and notes.
    pub fn text_x(self) -> f64 {
        self.inset + self.pad
    }
    /// Where a label's frame starts so that its first glyph stands on
    /// `text_x`: AppKit sets a label's text this far inside its frame.
    pub fn label_x(self) -> f64 {
        self.text_x() - LABEL_LEAD
    }
    /// Where trailing controls end, a row inset inside the card.
    pub fn control_right(self) -> f64 {
        self.width - self.inset - self.pad
    }
    pub fn card_width(self) -> f64 {
        self.width - 2. * self.inset
    }
    /// A text field: the value column, never more than half its card.
    pub fn text_width(self) -> f64 {
        self.value_column.min((self.card_width() / 2.).floor())
    }
    /// A number field: a third of the value column, so a size's two fields
    /// and their `×` stand inside it (`1920 × 1080`).
    pub fn number_width(self) -> f64 {
        (self.value_column / 3.).floor()
    }
    pub fn cross_width(self) -> f64 {
        (self.field / 2.).round()
    }
}
/// The inset AppKit gives a label's text inside its frame.
const LABEL_LEAD: f64 = 2.;
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
    /// A note under a row's title: its label, and the longest text it will
    /// show while the sheet is open (its height is kept for that).
    struct Note {
        label: Retained<NSTextField>,
        reserve: String,
    }
    /// One row of a card.
    struct Row {
        /// The title, or a control that is its own title (a radio, a checkbox).
        lead: Retained<NSView>,
        /// How far notes stand in from the title's edge (under a radio's title).
        indent: f64,
        notes: Vec<Note>,
        /// Controls from the trailing edge inwards, each at its own size.
        trailing: Vec<(Retained<NSView>, NSSize)>,
        /// A number's unit, or a small control in its place (a size's link).
        unit: Option<Retained<NSView>>,
        /// Numbers end before the sheet's unit column, so their right edges
        /// line up whatever their unit.
        numeric: bool,
    }
    enum Item {
        /// Text on the text column outside the cards: the title, a section
        /// heading, a line of introduction or warning.
        Line(Note),
        Row(Row),
    }
    /// Real AppKit sheet, retained with its controls until the owner closes it.
    /// Control keys are stable field/action IDs, never translated labels.
    /// Rows are gathered as the caller names them and laid out by `footer`.
    pub struct Sheet {
        pub window: Retained<NSWindow>,
        pub content: Retained<NSView>,
        pub fields: BTreeMap<String, Retained<NSTextField>>,
        pub buttons: BTreeMap<String, Retained<NSButton>>,
        pub controls: BTreeMap<String, Retained<NSControl>>,
        pub layout: SheetLayout,
        items: Vec<Item>,
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
    fn fitting(view: &NSView) -> NSSize {
        let size = view.fittingSize();
        NSSize::new(size.width.ceil(), size.height.ceil())
    }
    impl Sheet {
        pub fn new(title: &str, tokens: &Tokens, wide: bool, mtm: MainThreadMarker) -> Self {
            let layout = SheetLayout::resolve(tokens, wide);
            let rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(layout.width, layout.row));
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
            // The ground the app's other sheets stand on.
            window.setBackgroundColor(Some(&colour(tokens.colors.panel)));
            let content = NSView::initWithFrame(NSView::alloc(mtm), rect);
            window.setContentView(Some(&content));
            let mut sheet = Self {
                window,
                content,
                fields: BTreeMap::new(),
                buttons: BTreeMap::new(),
                controls: BTreeMap::new(),
                layout,
                items: Vec::new(),
                mtm,
                tokens: tokens.clone(),
            };
            let heading = sheet.line(title, tokens.typography.heading.into(), tokens.colors.text);
            heading.setFont(Some(&NSFont::systemFontOfSize_weight(
                tokens.typography.heading.into(),
                unsafe { NSFontWeightSemibold },
            )));
            sheet
        }
        fn wrapping(&self, text: &str, size: f64, rgb: u32) -> Retained<NSTextField> {
            let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), self.mtm);
            label.setFont(Some(&NSFont::systemFontOfSize(size)));
            label.setTextColor(Some(&colour(rgb)));
            label
        }
        /// A line on the text column, outside the cards.
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
            let label = self.wrapping(text, size, rgb);
            self.items.push(Item::Line(Note {
                label: label.clone(),
                reserve: reserve.into(),
            }));
            label
        }
        /// A quiet heading over the card of rows that follows it.
        pub fn heading(&mut self, text: &str) {
            let (size, rgb) = (
                f64::from(self.tokens.typography.label),
                self.tokens.colors.secondary,
            );
            let label = self.line(text, size, rgb);
            label.setFont(Some(&NSFont::systemFontOfSize_weight(size, unsafe {
                NSFontWeightMedium
            })));
        }
        /// A note under the last row's title: its description, or what is
        /// wrong with its value. Its height is kept for `reserve` too.
        pub fn note(&mut self, text: &str, reserve: &str, rgb: u32) -> Retained<NSTextField> {
            let size = self.tokens.typography.label.into();
            let Some(Item::Row(_)) = self.items.last() else {
                return self.line_reserving(text, reserve, size, rgb);
            };
            let label = self.wrapping(text, size, rgb);
            if let Some(Item::Row(row)) = self.items.last_mut() {
                row.notes.push(Note {
                    label: label.clone(),
                    reserve: reserve.into(),
                });
            }
            label
        }
        /// Starts a row with its title; controls are added to it after.
        fn titled(&mut self, label: &str) -> &mut Row {
            let title = NSTextField::labelWithString(&NSString::from_str(label), self.mtm);
            title.setFont(Some(&NSFont::systemFontOfSize(
                self.tokens.typography.title.into(),
            )));
            title.setTextColor(Some(&colour(self.tokens.colors.text)));
            title.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            self.items.push(Item::Row(Row {
                lead: title.into_super().into_super(),
                indent: 0.,
                notes: Vec::new(),
                trailing: Vec::new(),
                unit: None,
                numeric: false,
            }));
            match self.items.last_mut() {
                Some(Item::Row(row)) => row,
                _ => unreachable!(),
            }
        }
        /// A row whose control stands at its own fitting size.
        pub fn row(&mut self, label: &str, view: &NSView) {
            let mut size = fitting(view);
            if size.width < 1. {
                size.width = self.layout.text_width();
            }
            size.width = size.width.min(self.layout.value_column);
            size.height = size.height.max(self.layout.field);
            self.titled(label).trailing.push((view.into(), size));
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
                self.tokens.typography.label.into(),
            )));
            label.setTextColor(Some(&colour(self.tokens.colors.muted)));
            label
        }
        /// A small control in the last row's unit column (a size's aspect link).
        pub fn trailing(&mut self, view: &NSView) {
            if let Some(Item::Row(row)) = self.items.last_mut() {
                row.unit = Some(view.into());
            }
        }
        fn field_size(&self, field: &NSTextField, width: f64) -> NSSize {
            NSSize::new(width, fitting(field).height.max(self.layout.field))
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
            let size = self.field_size(&field, self.layout.text_width());
            self.titled(label)
                .trailing
                .push((field.clone().into_super().into_super(), size));
            self.fields.insert(id.into(), field);
        }
        /// A text field for source text (an expression): most of its row,
        /// in the fixed-width face.
        pub fn source(&mut self, id: &str, label: &str, value: &str) {
            self.text(id, label, value, false, true);
            let width = (self.layout.card_width() * 0.7).floor();
            if let Some(Item::Row(row)) = self.items.last_mut()
                && let Some((_, size)) = row.trailing.last_mut()
            {
                size.width = width;
            }
            if let Some(field) = self.fields.get(id) {
                field.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
                    self.tokens.typography.body.into(),
                    unsafe { NSFontWeightRegular },
                )));
            }
        }
        pub fn number(&mut self, id: &str, label: &str, value: &str, unit: &str, enabled: bool) {
            let field = self.number_field(id, label, value, enabled);
            let size = self.field_size(&field, self.layout.number_width());
            let unit = (!unit.is_empty()).then(|| self.small_label(unit));
            let row = self.titled(label);
            row.numeric = true;
            row.unit = unit.map(|u| u.into_super().into_super());
            row.trailing
                .push((field.clone().into_super().into_super(), size));
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
            let first = self.number_field(a, a_label, a_value, enabled);
            let second = self.number_field(b, b_label, b_value, enabled);
            let size = self.field_size(&first, self.layout.number_width());
            let cross = self.small_label("×");
            cross.setAlignment(NSTextAlignment::Center);
            let cross_size = NSSize::new(self.layout.cross_width(), fitting(&cross).height);
            let unit = (!unit.is_empty()).then(|| self.small_label(unit));
            let row = self.titled(label);
            row.numeric = true;
            row.unit = unit.map(|u| u.into_super().into_super());
            row.trailing
                .push((second.clone().into_super().into_super(), size));
            row.trailing
                .push((cross.into_super().into_super(), cross_size));
            row.trailing
                .push((first.clone().into_super().into_super(), size));
            self.fields.insert(a.into(), first);
            self.fields.insert(b.into(), second);
        }
        /// A colour well as a swatch at the trailing edge.
        pub fn swatch(&mut self, id: &str, label: &str, well: Retained<NSColorWell>) {
            well.setColorWellStyle(NSColorWellStyle::Minimal);
            well.setAccessibilityLabel(Some(&NSString::from_str(label)));
            well.setIdentifier(Some(&NSString::from_str(id)));
            let size = NSSize::new(
                2. * self.layout.field,
                self.layout.field + self.layout.gap / 2.,
            );
            self.titled(label)
                .trailing
                .push((well.clone().into_super().into_super(), size));
            self.controls.insert(id.into(), well.into_super());
        }
        /// A radio or checkbox whose title is its own: one row of a card.
        pub fn option(&mut self, id: &str, control: Retained<NSControl>) {
            control.setIdentifier(Some(&NSString::from_str(id)));
            self.items.push(Item::Row(Row {
                lead: control.clone().into_super(),
                // Notes start under the title, past the mark.
                indent: self.layout.field - self.layout.gap / 3.,
                notes: Vec::new(),
                trailing: Vec::new(),
                unit: None,
                numeric: false,
            }));
            self.controls.insert(id.into(), control);
        }
        /// A yes-or-no row: its title, and the system switch at the trailing edge.
        pub fn toggle(&mut self, id: &str, label: &str, on: bool, enabled: bool) {
            let switch = NSSwitch::initWithFrame(NSSwitch::alloc(self.mtm), NSRect::ZERO);
            switch.setState(if on {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            switch.setEnabled(enabled);
            self.control(id, label, switch.into_super());
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
        /// Lays the sheet out: the title, then lines and cards of rows down the
        /// one text column, then the buttons at the trailing edge in the spec's
        /// reading order (Cancel before the default), with a leading tertiary
        /// action. Call it last.
        pub fn footer(&self, order: &[&str], leading: Option<&str>) {
            let (l, t) = (self.layout, &self.tokens);
            let space = (
                f64::from(t.space.xs),
                f64::from(t.space.md),
                f64::from(t.space.xl),
            );
            let border = f64::from(t.metrics.border);
            // One unit column for the sheet: as wide as its widest unit.
            let unit = self
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Row(row) => row.unit.as_ref().map(|u| fitting(u).width),
                    Item::Line(_) => None,
                })
                .fold(0., f64::max);
            let measure = |note: &Note, width: f64| {
                note.label.setPreferredMaxLayoutWidth(width);
                let fit = |label: &NSTextField| {
                    label
                        .sizeThatFits(NSSize::new(width, f64::MAX))
                        .height
                        .ceil()
                };
                let shown = note.label.stringValue();
                let height = fit(&note.label);
                note.label
                    .setStringValue(&NSString::from_str(&note.reserve));
                let reserved = fit(&note.label);
                note.label.setStringValue(&shown);
                height.max(reserved)
            };
            // Frames from the top of the sheet; flipped once its height is known.
            let mut frames: Vec<(Retained<NSView>, NSRect)> = Vec::new();
            let mut cards: Vec<(f64, f64)> = Vec::new();
            let (mut y, mut card, mut after_line) = (l.inset, None::<f64>, false);
            for (index, item) in self.items.iter().enumerate() {
                match item {
                    Item::Line(note) => {
                        if let Some(top) = card.take() {
                            cards.push((top, y));
                            y += l.pad;
                        } else if index == 1 {
                            // Under the sheet's title.
                            y += space.1;
                        } else if after_line {
                            y += space.0;
                        }
                        let width = l.width - 2. * l.label_x();
                        let height = measure(note, width);
                        frames.push((
                            note.label.clone().into_super().into_super(),
                            NSRect::new(NSPoint::new(l.label_x(), y), NSSize::new(width, height)),
                        ));
                        y += height;
                        after_line = true;
                    }
                    Item::Row(row) => {
                        if card.is_none() {
                            // A card stands a row inset under the title, and
                            // closer under its own heading.
                            y += if index == 1 { l.pad } else { l.gap };
                            card = Some(y);
                        }
                        after_line = false;
                        let mut right = l.control_right();
                        let unit_x = right - unit;
                        if row.numeric && unit > 0. {
                            right = unit_x - l.gap;
                        }
                        let mut trailing = Vec::new();
                        for (view, size) in &row.trailing {
                            right -= size.width;
                            trailing.push((view.clone(), right, *size));
                            right -= l.gap;
                        }
                        // A title is a label; a radio or checkbox draws its own mark.
                        let lead_x = if row.indent > 0. {
                            l.text_x()
                        } else {
                            l.label_x()
                        };
                        let lead_width = right - (l.pad - l.gap) - lead_x;
                        let lead_height = fitting(&row.lead).height.max(1.);
                        let notes: Vec<f64> = row
                            .notes
                            .iter()
                            .map(|note| measure(note, lead_width - row.indent))
                            .collect();
                        let copy = lead_height + notes.iter().map(|h| h + border).sum::<f64>();
                        let least = if row.trailing.is_empty() && row.notes.is_empty() {
                            f64::from(t.metrics.row)
                        } else {
                            l.row
                        };
                        let height = (copy + 2. * l.gap).max(least);
                        let mut top = y + ((height - copy) / 2.).round();
                        frames.push((
                            row.lead.clone(),
                            NSRect::new(
                                NSPoint::new(lead_x, top),
                                NSSize::new(lead_width, lead_height),
                            ),
                        ));
                        top += lead_height;
                        for (note, note_height) in row.notes.iter().zip(notes) {
                            top += border;
                            frames.push((
                                note.label.clone().into_super().into_super(),
                                NSRect::new(
                                    NSPoint::new(lead_x + row.indent, top),
                                    NSSize::new(lead_width - row.indent, note_height),
                                ),
                            ));
                            top += note_height;
                        }
                        let middle = |size: NSSize| y + ((height - size.height) / 2.).round();
                        for (view, x, size) in trailing {
                            frames.push((view, NSRect::new(NSPoint::new(x, middle(size)), size)));
                        }
                        if let Some(view) = &row.unit {
                            let size = fitting(view);
                            frames.push((
                                view.clone(),
                                NSRect::new(NSPoint::new(unit_x, middle(size)), size),
                            ));
                        }
                        y += height;
                    }
                }
            }
            if let Some(top) = card {
                cards.push((top, y));
            }
            // The buttons: a Mac sheet's, at the trailing edge under the form.
            let tallest = self
                .buttons
                .values()
                .map(|b| b.frame().size.height)
                .fold(l.field, f64::max);
            y += space.2;
            let mut x = l.width - l.inset;
            for id in order.iter().rev() {
                let b = &self.buttons[*id];
                let mut size = b.frame().size;
                size.width = size.width.max(2. * l.row);
                x -= size.width;
                frames.push((
                    b.clone().into_super().into_super(),
                    NSRect::new(NSPoint::new(x, y), size),
                ));
                x -= space.1;
            }
            if let Some(id) = leading {
                let b = &self.buttons[id];
                frames.push((
                    b.clone().into_super().into_super(),
                    NSRect::new(NSPoint::new(l.inset, y), b.frame().size),
                ));
            }
            let total = (y + tallest + l.inset).ceil();
            let size = NSSize::new(l.width, total);
            self.window.setContentSize(size);
            self.content.setFrameSize(size);
            let flipped = |r: NSRect| {
                NSRect::new(
                    NSPoint::new(
                        r.origin.x.round(),
                        (total - r.origin.y - r.size.height).round(),
                    ),
                    r.size,
                )
            };
            for (top, bottom) in cards {
                let card = NSBox::new(self.mtm);
                card.setBoxType(NSBoxType::Custom);
                card.setTitlePosition(NSTitlePosition::NoTitle);
                card.setBorderWidth(0.);
                card.setCornerRadius(t.metrics.group_radius.into());
                card.setFillColor(&colour(t.colors.fill));
                card.setFrame(flipped(NSRect::new(
                    NSPoint::new(l.inset, top),
                    NSSize::new(l.card_width(), bottom - top),
                )));
                self.content.addSubview(&card);
            }
            for (view, frame) in frames {
                view.setFrame(flipped(frame));
                self.content.addSubview(&view);
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
                    // One text column, a row inset inside the cards; controls
                    // end the same inset inside the other edge.
                    assert_eq!(g.text_x(), g.inset + f64::from(t.metrics.inset));
                    assert_eq!(g.text_x() + g.control_right(), g.width);
                    assert_eq!(g.card_width() + 2. * g.inset, g.width);
                    // A size's two fields and their cross fit the value column.
                    assert!(2. * g.number_width() + g.cross_width() + 2. * g.gap <= g.value_column);
                    assert!(g.text_width() <= g.card_width() / 2.);
                }
            }
        }
    }
}
