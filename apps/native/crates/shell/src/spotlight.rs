//! Command palette chrome and reusable result decorations.
use super::{native_color, popup_label, symbol};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, rc::Retained, runtime::AnyObject, sel};
use objc2_app_kit::*;
use objc2_foundation::{
    NSAttributedStringKey, NSDictionary, NSMutableAttributedString, NSPoint, NSRange, NSRect,
    NSSize, NSString,
};
use pm_design::{Scheme, Tokens};
use std::collections::HashMap;

pub(super) const WIDTH: f32 = 540.;
pub(super) const RADIUS: f32 = 20.;
pub(super) const ROW_RADIUS: f64 = 10.;
pub(super) const HEADER: f64 = 52.;
pub(super) const FOOTER: f64 = 32.;
pub(super) const QUICK_PICKS: usize = 9;

fn frame(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn box_view(mtm: MainThreadMarker, color: &NSColor) -> Retained<NSBox> {
    let view = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::ZERO);
    view.setBoxType(NSBoxType::Custom);
    view.setBorderWidth(0.);
    view.setFillColor(color);
    view.setAccessibilityElement(false);
    view
}

fn configured_symbol(name: &str, config: &NSImageSymbolConfiguration) -> Retained<NSImage> {
    let image = symbol(name, "");
    image.imageWithSymbolConfiguration(config).unwrap_or(image)
}

const EFFECT_SYMBOL: &str = "sparkles";
const LAYER_SYMBOL: &str = "square.3.layers.3d";

pub(super) struct Chrome {
    magnifier: Retained<NSImageView>,
    pub(super) clear: Retained<NSButton>,
    header_line: Retained<NSBox>,
    footer: Retained<NSBox>,
    footer_line: Retained<NSBox>,
    hints: Retained<NSTextField>,
    pub(super) count: Retained<NSTextField>,
    totals: Option<(usize, usize)>,
    geometry: Option<(f64, f64, f64)>,
}

impl Chrome {
    pub(super) fn new(mtm: MainThreadMarker, body: &NSView, t: &Tokens, pixel: f64) -> Self {
        let magnifier = NSImageView::imageViewWithImage(
            &configured_symbol(
                "magnifyingglass",
                &NSImageSymbolConfiguration::configurationWithPointSize_weight(15., unsafe {
                    NSFontWeightRegular
                }),
            ),
            mtm,
        );
        magnifier.setAccessibilityElement(false);
        magnifier.setContentTintColor(Some(&native_color(t.colors.muted)));
        let clear = unsafe {
            NSButton::buttonWithImage_target_action(
                &symbol("xmark.circle.fill", ""),
                None,
                Some(sel!(clearSearch:)),
                mtm,
            )
        };
        clear.setBordered(false);
        clear.setContentTintColor(Some(&native_color(t.colors.muted)));
        clear.setSymbolConfiguration(Some(
            &NSImageSymbolConfiguration::configurationWithPointSize_weight(16., unsafe {
                NSFontWeightRegular
            }),
        ));
        clear.setAccessibilityLabel(Some(&NSString::from_str("Clear search")));
        clear.setHidden(true);

        let line = native_color(t.colors.separator).colorWithAlphaComponent(0.7);
        let header_line = box_view(mtm, &line);
        let footer_line = box_view(mtm, &line);
        let footer = box_view(
            mtm,
            &native_color(0x000000).colorWithAlphaComponent(if t.scheme == Scheme::Light {
                0.035
            } else {
                0.13
            }),
        );
        let hints = popup_label("", 11., t.colors.muted, mtm);
        let hint_text = "↵ Run    ↑↓ Navigate    ⌘ 1–9    esc Close";
        let title = NSMutableAttributedString::initWithString(
            NSMutableAttributedString::alloc(),
            &NSString::from_str(hint_text),
        );
        let regular = NSFont::systemFontOfSize(11.);
        let full = NSRange::new(0, hint_text.encode_utf16().count());
        unsafe {
            title.addAttributes_range(
                &ink_attributes(&regular, &native_color(t.colors.muted)),
                full,
            );
        }
        let brighter = native_color(t.colors.secondary);
        for glyph in ["↵", "↑↓", "⌘ 1–9", "esc"] {
            let start = hint_text.find(glyph).unwrap();
            let range = NSRange::new(
                hint_text[..start].encode_utf16().count(),
                glyph.encode_utf16().count(),
            );
            unsafe {
                title.addAttribute_value_range(NSForegroundColorAttributeName, &brighter, range)
            };
        }
        hints.setAttributedStringValue(&title);
        let count = popup_label("", 11., t.colors.muted, mtm);
        count.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
            11.,
            unsafe { NSFontWeightRegular },
        )));
        count.setAlignment(NSTextAlignment::Right);
        count.setAccessibilityElement(true);
        for view in [
            &*magnifier as &NSView,
            &clear,
            &header_line,
            &footer,
            &footer_line,
            &hints,
            &count,
        ] {
            body.addSubview(view);
        }
        // Resize with the material. Its inner border leaves the content flush.
        body.setWantsLayer(true);
        if let Some(layer) = body.layer() {
            layer.setCornerRadius(f64::from(RADIUS));
            layer.setMasksToBounds(true);
            layer.setBorderWidth(pixel);
            let rim = if t.scheme == Scheme::Light {
                t.colors.separator
            } else {
                0xffffff
            };
            let rim = native_color(rim).colorWithAlphaComponent(0.16);
            layer.setBorderColor(Some(&rim.CGColor()));
        }
        Self {
            magnifier,
            clear,
            header_line,
            footer,
            footer_line,
            hints,
            count,
            totals: None,
            geometry: None,
        }
    }

    pub(super) fn layout(&mut self, width: f64, height: f64, pixel: f64) {
        let geometry = (width, height, pixel);
        if self.geometry == Some(geometry) {
            return;
        }
        self.geometry = Some(geometry);
        let header_y = height - HEADER;
        self.magnifier
            .setFrame(frame(16., header_y + 16., 20., 20.));
        self.clear
            .setFrame(frame(width - 34., header_y + 17., 18., 18.));
        self.header_line.setFrame(frame(0., header_y, width, pixel));
        self.footer.setFrame(frame(0., 0., width, FOOTER));
        self.footer_line
            .setFrame(frame(0., FOOTER - pixel, width, pixel));
        self.hints.setFrame(frame(16., 8., 300., 16.));
        self.count.setFrame(frame(width - 198., 8., 182., 16.));
    }

    pub(super) fn update(&mut self, query: &str, shown: usize, unlisted: usize) {
        if self.clear.isHidden() != query.is_empty() {
            self.clear.setHidden(query.is_empty());
        }
        if self.totals != Some((shown, unlisted)) {
            let text = if unlisted > 0 {
                format!("{shown} shown · {unlisted} more")
            } else if shown == 1 {
                "1 result".to_owned()
            } else {
                format!("{shown} results")
            };
            self.count.setStringValue(&NSString::from_str(&text));
            self.totals = Some((shown, unlisted));
        }
    }
}

type Attributes = NSDictionary<NSAttributedStringKey, AnyObject>;

fn ink_attributes(font: &NSFont, color: &NSColor) -> Retained<Attributes> {
    NSDictionary::from_slices(
        &[unsafe { NSFontAttributeName }, unsafe {
            NSForegroundColorAttributeName
        }],
        &[font as &AnyObject, color as &AnyObject],
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Neutral,
    Danger,
    Effect,
    Layer,
}

fn row_symbol(command: &str, category: &str) -> (&'static str, Tone) {
    let command = command.split('\u{1f}').next().unwrap_or_default();
    match (command, category) {
        (
            "delete"
            | "rippleDelete"
            | "liftWorkArea"
            | "extractWorkArea"
            | "deleteWorkspace"
            | "deleteKeyframes"
            | "removeEffect"
            | "viewer.deleteRendered",
            _,
        ) => ("trash", Tone::Danger),
        ("duplicate" | "contextCopy", _) => ("doc.on.doc", Tone::Neutral),
        (
            "split" | "contextCut" | "splitTextWords" | "splitTextCharacters" | "splitTextLines",
            _,
        ) => ("scissors", Tone::Neutral),
        ("undo" | "contextUndo", _) => ("arrow.uturn.backward", Tone::Neutral),
        ("redo" | "contextRedo", _) => ("arrow.uturn.forward", Tone::Neutral),
        ("deselect" | "contextSelectAll", _) => ("square.dashed", Tone::Neutral),
        ("keyboardShortcuts", _) => ("keyboard", Tone::Neutral),
        ("palette.addEffect", _) | (_, "Effect") => (EFFECT_SYMBOL, Tone::Effect),
        ("palette.selectLayer", _) | (_, "Layer") => (LAYER_SYMBOL, Tone::Layer),
        ("export" | "exportCaptions", _) | (_, "Export") => ("square.and.arrow.up", Tone::Neutral),
        (_, "Panels") => ("sidebar.left", Tone::Neutral),
        (_, "Workspace") => ("rectangle.3.group", Tone::Neutral),
        (_, "View") => ("eye", Tone::Neutral),
        _ => ("command", Tone::Neutral),
    }
}

/// A case-insensitive literal or subsequence match in AppKit's UTF-16 coordinates.
fn match_ranges(title: &str, query: &str) -> Vec<NSRange> {
    if query.is_empty() {
        return Vec::new();
    }
    if title.is_ascii() {
        let lower = title.to_ascii_lowercase();
        if let Some(start) = lower.find(query) {
            return vec![NSRange::new(start, query.len())];
        }
        return subsequence(
            lower
                .chars()
                .enumerate()
                .map(|(i, c)| (c, NSRange::new(i, 1))),
            query,
        );
    }

    let lower = title.to_lowercase();
    // One lowered character may expand into several. Each one still
    // points to the full original character, including surrogate pairs.
    let mut offset = 0;
    let source = title.chars().flat_map(|c| {
        let range = NSRange::new(offset, c.len_utf16());
        offset += range.length;
        std::iter::repeat_n(range, c.to_lowercase().count())
    });
    let mut mapped = lower
        .char_indices()
        .zip(source)
        .map(|((i, c), r)| (i, c, r));
    if let Some(start) = lower.find(query) {
        let mut literal = mapped
            .by_ref()
            .skip_while(|&(i, _, _)| i < start)
            .take_while(|&(i, _, _)| i < start + query.len());
        if let Some((_, _, first)) = literal.next() {
            let last = literal.last().map_or(first, |(_, _, r)| r);
            return vec![NSRange::new(
                first.location,
                last.location + last.length - first.location,
            )];
        }
    }
    subsequence(mapped.map(|(_, c, r)| (c, r)), query)
}

fn subsequence(chars: impl Iterator<Item = (char, NSRange)>, query: &str) -> Vec<NSRange> {
    let mut needed = query.chars().filter(|c| !c.is_whitespace());
    let mut next = needed.next();
    let mut ranges: Vec<NSRange> = Vec::new();
    for (c, range) in chars {
        if Some(c) == next {
            if let Some(last) = ranges.last_mut()
                && range.location <= last.location + last.length
            {
                last.length = range.location + range.length - last.location;
            } else {
                ranges.push(range);
            }
            next = needed.next();
            if next.is_none() {
                return ranges;
            }
        }
    }
    Vec::new()
}

struct RowInk {
    soft: Retained<Attributes>,
    strong: Retained<Attributes>,
}

pub(super) struct RowTheme {
    ink: [RowInk; 3],
    primary: Retained<NSColor>,
    secondary: Retained<NSColor>,
    danger: Retained<NSColor>,
    selected_danger: Retained<NSColor>,
    effect: Retained<NSColor>,
    layer: Retained<NSColor>,
    accent: Retained<NSColor>,
    image_config: Retained<NSImageSymbolConfiguration>,
    images: HashMap<&'static str, Retained<NSImage>>,
}

impl RowTheme {
    pub(super) fn new(t: &Tokens) -> Self {
        let primary = native_color(t.colors.text);
        let danger = native_color(t.colors.danger);
        let selected_danger = native_color(t.colors.selected_danger);
        let size = f64::from(t.typography.body);
        let regular = NSFont::systemFontOfSize(size);
        let bold = NSFont::systemFontOfSize_weight(size, unsafe { NSFontWeightSemibold });
        let ink = |color: &NSColor| RowInk {
            soft: ink_attributes(&regular, &color.colorWithAlphaComponent(0.82)),
            strong: ink_attributes(&bold, color),
        };
        let (effect, layer) = if t.scheme == Scheme::Light {
            (0x7052a5, 0x387064)
        } else {
            (0xbba4e7, 0x8bc2b7)
        };
        let image_config =
            NSImageSymbolConfiguration::configurationWithPointSize_weight(14., unsafe {
                NSFontWeightRegular
            });
        // Effects and layers are usually absent until the first search.
        let images = [EFFECT_SYMBOL, LAYER_SYMBOL]
            .into_iter()
            .map(|name| (name, configured_symbol(name, &image_config)))
            .collect();
        Self {
            ink: [ink(&primary), ink(&danger), ink(&selected_danger)],
            primary,
            secondary: native_color(t.colors.secondary),
            danger,
            selected_danger,
            effect: native_color(effect),
            layer: native_color(layer),
            accent: native_color(t.colors.accent),
            image_config,
            images,
        }
    }

    fn image(&mut self, name: &'static str) -> &NSImage {
        self.images
            .entry(name)
            .or_insert_with(|| configured_symbol(name, &self.image_config))
    }

    fn icon_ink(&self, tone: Tone, lit: bool) -> &NSColor {
        match (tone, lit) {
            (Tone::Danger, true) => &self.selected_danger,
            (Tone::Danger, false) => &self.danger,
            (Tone::Effect, _) => &self.effect,
            (Tone::Layer, _) => &self.layer,
            (Tone::Neutral, true) => &self.primary,
            (Tone::Neutral, false) => &self.secondary,
        }
    }
}

pub(super) struct RowDecoration {
    title: Retained<NSTextField>,
    icon: Retained<NSImageView>,
    text: String,
    ranges: Vec<NSRange>,
    searched: bool,
    lit: bool,
    glyph: &'static str,
    tone: Tone,
}

impl RowDecoration {
    pub(super) fn new(list: &NSView, below: Option<&NSView>, t: &Tokens) -> Self {
        let mtm = MainThreadMarker::from(list);
        let title = popup_label("", t.typography.body, t.colors.text, mtm);
        let icon = NSImageView::initWithFrame(NSImageView::alloc(mtm), NSRect::ZERO);
        icon.setAccessibilityElement(false);
        for view in [&*title as &NSView, &icon] {
            let order = if below.is_some() {
                NSWindowOrderingMode::Below
            } else {
                NSWindowOrderingMode::Above
            };
            list.addSubview_positioned_relativeTo(view, order, below);
        }
        Self {
            title,
            icon,
            text: String::new(),
            ranges: Vec::new(),
            searched: false,
            lit: false,
            glyph: "",
            tone: Tone::Neutral,
        }
    }

    pub(super) fn update(
        &mut self,
        theme: &mut RowTheme,
        text: &str,
        command: &str,
        category: &str,
        query: &str,
        lit: bool,
    ) {
        let (glyph, tone) = row_symbol(command, category);
        let new_icon = self.glyph != glyph;
        if new_icon {
            self.icon.setImage(Some(theme.image(glyph)));
            self.glyph = glyph;
        }
        if new_icon || self.tone != tone || self.lit != lit {
            self.icon
                .setContentTintColor(Some(theme.icon_ink(tone, lit)));
        }
        let ranges = match_ranges(text, query);
        let searched = !query.is_empty();
        if self.text != text
            || self.ranges != ranges
            || self.searched != searched
            || (self.tone == Tone::Danger) != (tone == Tone::Danger)
            || (tone == Tone::Danger && self.lit != lit)
        {
            text.clone_into(&mut self.text);
            self.ranges = ranges;
            self.searched = searched;
            self.tone = tone;
            self.lit = lit;
            self.draw_title(theme);
        }
        self.tone = tone;
        self.lit = lit;
    }

    pub(super) fn light(&mut self, theme: &RowTheme, lit: bool) {
        if self.lit != lit {
            self.lit = lit;
            self.icon
                .setContentTintColor(Some(theme.icon_ink(self.tone, lit)));
            if self.tone == Tone::Danger {
                self.draw_title(theme);
            }
        }
    }

    fn draw_title(&self, theme: &RowTheme) {
        if !self.searched {
            let color = if self.tone == Tone::Danger {
                theme.icon_ink(self.tone, self.lit)
            } else {
                &theme.primary
            };
            self.title.setStringValue(&NSString::from_str(&self.text));
            if self.title.textColor().as_deref() != Some(color) {
                self.title.setTextColor(Some(color));
            }
            return;
        }
        let ink = &theme.ink[match (self.tone, self.lit) {
            (Tone::Danger, true) => 2,
            (Tone::Danger, false) => 1,
            _ => 0,
        }];
        let title = unsafe {
            NSMutableAttributedString::initWithString_attributes(
                NSMutableAttributedString::alloc(),
                &NSString::from_str(&self.text),
                Some(&ink.soft),
            )
        };
        for &range in &self.ranges {
            unsafe { title.addAttributes_range(&ink.strong, range) };
        }
        self.title.setAttributedStringValue(&title);
    }

    pub(super) fn layout(&self, rect: NSRect, title_height: f64) {
        let x = rect.origin.x + 8.;
        let centre = rect.origin.y + rect.size.height / 2.;
        for (view, target) in [
            (&*self.icon as &NSView, frame(x, centre - 10., 20., 20.)),
            (
                &self.title,
                frame(
                    x + 30.,
                    (centre - title_height / 2.).floor(),
                    rect.size.width - 76.,
                    title_height,
                ),
            ),
        ] {
            if view.frame() != target {
                view.setFrame(target);
            }
        }
    }

    pub(super) fn set_hidden(&self, hidden: bool) {
        if self.title.isHidden() != hidden {
            self.title.setHidden(hidden);
            self.icon.setHidden(hidden);
        }
    }

    pub(super) fn remove(&self) {
        self.title.removeFromSuperview();
        self.icon.removeFromSuperview();
    }
}

pub(super) struct Badge {
    pub(super) label: Retained<NSTextField>,
    lit: Option<bool>,
}

impl Badge {
    pub(super) fn new(place: usize, list: &NSView, below: &NSView, t: &Tokens) -> Self {
        let label = popup_label(
            &(place + 1).to_string(),
            11.,
            t.colors.secondary,
            MainThreadMarker::from(list),
        );
        label.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
            11.,
            unsafe { NSFontWeightSemibold },
        )));
        label.setAlignment(NSTextAlignment::Center);
        // A layer on the row translates with the digit and has the same hit target as the row.
        label.setWantsLayer(true);
        label.layer().unwrap().setCornerRadius(6.);
        label.setHidden(true);
        list.addSubview_positioned_relativeTo(&label, NSWindowOrderingMode::Below, Some(below));
        Self { label, lit: None }
    }

    pub(super) fn show(&mut self, rect: NSRect, theme: &RowTheme, lit: bool) {
        let target = frame(
            rect.origin.x + rect.size.width - 28.,
            rect.origin.y + 7.,
            20.,
            20.,
        );
        if self.label.frame() != target {
            self.label.setFrame(target);
        }
        if self.lit != Some(lit) {
            let (ink, wash) = if lit {
                (&theme.accent, 0.16)
            } else {
                (&theme.secondary, 0.10)
            };
            self.label.setTextColor(Some(ink));
            let fill = ink.colorWithAlphaComponent(wash);
            if let Some(layer) = self.label.layer() {
                layer.setBackgroundColor(Some(&fill.CGColor()));
            }
            self.lit = Some(lit);
        }
        if self.label.isHidden() {
            self.label.setHidden(false);
        }
    }
}

pub(super) fn command_held(flags: NSEventModifierFlags) -> bool {
    flags.contains(NSEventModifierFlags::Command)
        && !flags.intersects(
            NSEventModifierFlags::Control
                | NSEventModifierFlags::Option
                | NSEventModifierFlags::Shift,
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_matches_use_utf16_offsets_and_only_complete_matches() {
        assert_eq!(match_ranges("Group Layers", "lay"), [NSRange::new(6, 3)]);
        assert_eq!(
            match_ranges("Go to Start", "gtst"),
            [NSRange::new(0, 1), NSRange::new(3, 1), NSRange::new(6, 2),]
        );
        assert_eq!(match_ranges("🎞 İris", "i\u{307}r"), [NSRange::new(3, 2)]);
        assert!(match_ranges("Group Layers", "gm").is_empty());
    }
}
