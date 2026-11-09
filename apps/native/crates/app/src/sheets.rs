//! Real AppKit sheets for app surfaces. Controls only enqueue their stable ids;
//! the caller reads values and validates on its own turn, so no control callback
//! ever does work. One presenter serves every form on the sheet_layout recipe.
use crate::dialogs::{NativePiece, Surface, sheet_layout::native::Sheet};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
    sel,
};
use objc2_app_kit::{
    NSColorWell, NSColorWellStyle, NSControl, NSControlTextEditingDelegate, NSPopUpButton,
    NSTextField, NSTextFieldDelegate, NSUserInterfaceItemIdentification, NSWindow,
};
use objc2_foundation::{NSNotification, NSRect, NSSize, NSString};
use std::collections::BTreeMap;

pub struct TargetState {
    tx: async_channel::Sender<String>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSheetTarget"]
    #[ivars = TargetState]
    struct Target;
    unsafe impl NSObjectProtocol for Target {}
    unsafe impl NSControlTextEditingDelegate for Target {
        #[unsafe(method(controlTextDidChange:))]
        fn changed(&self, notification: &NSNotification) {
            let id = notification
                .object()
                .and_then(|o| o.downcast::<NSControl>().ok())
                .and_then(|c| c.identifier());
            if let Some(id) = id {
                let _ = self.ivars().tx.try_send(format!("changed:{id}"));
            }
        }
    }
    unsafe impl NSTextFieldDelegate for Target {}
    impl Target {
        #[unsafe(method(control:))]
        fn control(&self, sender: &NSControl) {
            if let Some(id) = sender.identifier() {
                let _ = self.ivars().tx.try_send(id.to_string());
            }
        }
    }
);
/// A presented sheet, its target and its stream of control ids.
pub struct OpenSheet {
    pub sheet: Sheet,
    owner: Retained<NSWindow>,
    _target: Retained<Target>,
    /// What each colour well showed when the sheet opened.
    seeded: BTreeMap<String, String>,
    /// Each field's description line, which can change while the sheet is open.
    descriptions: BTreeMap<String, Retained<NSTextField>>,
    pub actions: async_channel::Receiver<String>,
}
impl OpenSheet {
    pub fn present(
        (sheet, descriptions): (Sheet, BTreeMap<String, Retained<NSTextField>>),
        owner: &NSWindow,
        first: Option<&str>,
    ) -> Self {
        let mtm = MainThreadMarker::new().unwrap();
        let (tx, actions) = async_channel::unbounded();
        let target: Retained<Target> = unsafe {
            msg_send![
                super(Target::alloc(mtm).set_ivars(TargetState { tx })),
                init
            ]
        };
        unsafe {
            sheet.bind(target.as_ref() as &AnyObject, sel!(control:));
        }
        for field in sheet.fields.values() {
            unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*target))) };
        }
        sheet.present(owner, first);
        let seeded = sheet
            .controls
            .iter()
            .filter_map(|(id, control)| Some((id.clone(), well_colour(control)?)))
            .collect();
        Self {
            sheet,
            owner: owner.into(),
            _target: target,
            seeded,
            descriptions,
            actions,
        }
    }
    /// Rewrites a field's description line (its reserved height stays).
    pub fn describe(&self, id: &str, text: &str, rgb: u32) {
        if let Some(label) = self.descriptions.get(id) {
            label.setStringValue(&NSString::from_str(text));
            label.setTextColor(Some(&crate::dialogs::sheet_layout::native::colour(rgb)));
        }
    }
    pub fn text(&self, id: &str) -> String {
        self.sheet
            .fields
            .get(id)
            .map(|f| f.stringValue().to_string())
            .unwrap_or_default()
    }
    pub fn popup(&self, id: &str) -> Option<usize> {
        let control = self.sheet.controls.get(id)?.clone();
        let popup = control.downcast::<NSPopUpButton>().ok()?;
        usize::try_from(popup.indexOfSelectedItem()).ok()
    }
    pub fn select(&self, id: &str, index: usize) {
        if let Some(popup) = self
            .sheet
            .controls
            .get(id)
            .and_then(|c| c.clone().downcast::<NSPopUpButton>().ok())
        {
            popup.selectItemAtIndex(index as isize);
        }
    }
    pub fn set_text(&self, id: &str, value: &str) {
        if let Some(field) = self.sheet.fields.get(id) {
            field.setStringValue(&NSString::from_str(value));
        }
    }
    pub fn enable(&self, id: &str, enabled: bool) {
        if let Some(button) = self.sheet.buttons.get(id) {
            button.setEnabled(enabled);
        }
    }
    /// The well's colour as `#RRGGBB`, only once the person has changed it: an
    /// untouched well never rewrites the authored string it was seeded from.
    pub fn changed_colour(&self, id: &str) -> Option<String> {
        let colour = well_colour(self.sheet.controls.get(id)?)?;
        (self.seeded.get(id) != Some(&colour)).then_some(colour)
    }
    pub fn close(&self) {
        self.owner.endSheet(&self.sheet.window);
        self.sheet.window.orderOut(None);
    }
}
fn well_colour(control: &NSControl) -> Option<String> {
    let well = control.downcast_ref::<NSColorWell>()?;
    crate::colour_panel::hex(&well.color())
}
/// `#RGB` or `#RRGGBB`. Anything else leaves a well unseeded.
pub fn hex_colour(hex: &str) -> Option<u32> {
    let digits = hex.strip_prefix('#')?;
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => Some(value),
        3 => {
            let (r, g, b) = ((value >> 8) & 15, (value >> 4) & 15, value & 15);
            Some(r * 0x110000 + g * 0x1100 + b * 0x11)
        }
        _ => None,
    }
}
/// The token rows a surface takes, so the footer sits below all of them.
fn rows(surface: &Surface) -> usize {
    let controls: usize = surface
        .fields
        .iter()
        .map(|f| match f.piece {
            NativePiece::NumberField => f.values.len().max(1),
            _ => 1,
        })
        .sum();
    let lines = surface
        .fields
        .iter()
        .map(|f| usize::from(f.error.is_some()) + usize::from(f.description.is_some()))
        .sum::<usize>();
    controls + lines + surface.groups.len() + usize::from(surface.line.is_some())
}
/// `Surface` forms: labelled controls in spec order and its footer buttons.
/// A multi-value number field makes one well per value: `{id}.0`, `{id}.1`.
/// Returns the sheet and its fields' description lines.
pub fn from_surface(
    surface: &Surface,
    tokens: &pm_design::Tokens,
) -> (Sheet, BTreeMap<String, Retained<NSTextField>>) {
    let mtm = MainThreadMarker::new().unwrap();
    let mut descriptions = BTreeMap::new();
    let mut sheet = Sheet::new(&surface.title, rows(surface), tokens, false, mtm);
    if let Some(line) = &surface.line {
        sheet.line(line, tokens.typography.body.into(), tokens.colors.secondary);
    }
    for field in &surface.fields {
        let value = field.values.first().map_or("", String::as_str);
        if let Some(group) = surface
            .groups
            .iter()
            .find(|g| g.fields.first() == Some(&field.id.as_str()))
        {
            sheet.line(
                group.title,
                tokens.typography.body.into(),
                tokens.colors.text,
            );
        }
        match field.piece {
            NativePiece::NumberField if field.values.len() > 1 => {
                for (index, value) in field.values.iter().enumerate() {
                    let id = format!("{}.{index}", field.id);
                    let label = if index == 0 { field.label.as_str() } else { "" };
                    sheet.number(&id, label, value, &field.unit, field.enabled);
                }
            }
            NativePiece::NumberField => {
                sheet.number(&field.id, &field.label, value, &field.unit, field.enabled)
            }
            NativePiece::ColourWell => {
                let well = NSColorWell::initWithFrame(NSColorWell::alloc(mtm), NSRect::ZERO);
                well.setColorWellStyle(NSColorWellStyle::Minimal);
                if let Some(rgb) = hex_colour(value) {
                    well.setColor(&crate::dialogs::sheet_layout::native::colour(rgb));
                }
                // A minimal well is a swatch, not a value column-wide bar.
                let swatch = NSSize::new(
                    f64::from(2. * tokens.metrics.field),
                    f64::from(tokens.metrics.field),
                );
                sheet.control(&field.id, &field.label, well.clone().into_super());
                well.setFrameSize(swatch);
            }
            NativePiece::PopUpButton => {
                let popup = NSPopUpButton::initWithFrame_pullsDown(
                    NSPopUpButton::alloc(mtm),
                    NSRect::ZERO,
                    false,
                );
                for item in &field.values {
                    popup.addItemWithTitle(&NSString::from_str(item));
                }
                popup.selectItemAtIndex(field.selected as isize);
                popup.setEnabled(field.enabled);
                sheet.control(&field.id, &field.label, popup.into_super().into_super());
            }
            _ => sheet.text(&field.id, &field.label, value, false, field.enabled),
        }
        if let (Some(placeholder), Some(control)) = (field.placeholder, sheet.fields.get(&field.id))
        {
            control.setPlaceholderString(Some(&NSString::from_str(placeholder)));
        }
        if let Some(text) = &field.description {
            // Reserve the longest thing the line says (a duration's cut-off warning).
            let reserve = if field.id == "duration" {
                crate::dialogs::cut_many(99)
            } else {
                text.clone()
            };
            let rgb = if field.warning {
                tokens.colors.warning
            } else {
                tokens.colors.muted
            };
            let label = sheet.line_reserving(text, &reserve, tokens.typography.minimum.into(), rgb);
            descriptions.insert(field.id.clone(), label);
        }
        if let Some(error) = field.error {
            sheet.line(
                error,
                tokens.typography.minimum.into(),
                tokens.colors.danger,
            );
        }
    }
    let mut order = Vec::new();
    for button in &surface.buttons {
        sheet.button(
            button.id,
            button.label,
            button.enabled,
            button.default,
            button.label == "Cancel",
        );
        order.push(button.id);
    }
    sheet.footer(&order, None);
    (sheet, descriptions)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogs::{CompositionDraft, DialogId, MixedNumber, VelocityDraft, WindowId};
    #[test]
    fn hex_seeds_only_what_it_can_read() {
        assert_eq!(hex_colour("#1a1a2e"), Some(0x1a1a2e));
        assert_eq!(hex_colour("#fff"), Some(0xffffff));
        assert_eq!(hex_colour("#FFFFFF00"), None);
        assert_eq!(hex_colour("red"), None);
        assert_eq!(hex_colour(""), None);
    }
    #[test]
    fn rows_count_every_line_a_surface_draws() {
        let draft = CompositionDraft {
            name: "Comp".into(),
            width: 1920.,
            height: 1080.,
            fps: 30.,
            duration: 10.,
            background: "#000000".into(),
        };
        let (id, owner) = (DialogId("sheet".into()), WindowId("window".into()));
        // Six fields, Size takes two rows, and Duration has its detail line.
        let plain = draft.surface(id.clone(), owner.clone(), false, 0);
        assert_eq!(rows(&plain), 8);
        let warned = draft.surface(id.clone(), owner.clone(), true, 2);
        assert_eq!(rows(&warned), 8);
        let number = MixedNumber::new(&[1.]);
        let velocity = VelocityDraft {
            incoming_speed: number.clone(),
            incoming_influence: number.clone(),
            outgoing_speed: number.clone(),
            outgoing_influence: number,
        };
        // Four fields, two headings and the keyframe count.
        assert_eq!(rows(&velocity.surface(id, owner, 1, "")), 7);
    }
}
