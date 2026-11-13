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
    NSColorWell, NSControl, NSControlTextEditingDelegate, NSEventModifierFlags, NSPopUpButton,
    NSTextDelegate, NSTextField, NSTextFieldDelegate, NSTextView, NSTextViewDelegate,
    NSUserInterfaceItemIdentification, NSWindow,
};
use objc2_foundation::{NSNotification, NSRect, NSString};
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
    unsafe impl NSTextDelegate for Target {
        #[unsafe(method(textDidChange:))]
        fn text_changed(&self, notification: &NSNotification) {
            let id = notification
                .object()
                .and_then(|o| o.downcast::<NSTextView>().ok())
                .and_then(|v| v.identifier());
            if let Some(id) = id {
                let _ = self.ivars().tx.try_send(format!("changed:{id}"));
            }
        }
    }
    unsafe impl NSTextViewDelegate for Target {}
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
        let cancelled = tx.clone();
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
        for editor in sheet.editors.values() {
            editor.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        }
        // Never begun (held, then dropped): the loop hears Cancel.
        sheet.present(owner, first, move || {
            let _ = cancelled.try_send("cancel".into());
        });
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
        if let Some(editor) = self.sheet.editors.get(id) {
            return editor.string().to_string();
        }
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
        gpui_macos::child_window::forget_sheet(&self.sheet.window);
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
/// The lines an expression's editor shows before it scrolls.
const EXPRESSION_LINES: usize = 7;
/// `Surface` forms: labelled controls in spec order and its footer buttons.
/// A multi-value number field makes one well per value: `{id}.0`, `{id}.1`.
/// Returns the sheet and its fields' description lines.
pub fn from_surface(
    surface: &Surface,
    tokens: &pm_design::Tokens,
) -> (Sheet, BTreeMap<String, Retained<NSTextField>>) {
    let mtm = MainThreadMarker::new().unwrap();
    let mut descriptions = BTreeMap::new();
    // An expression editor is the wide sheet: its field holds a line of source.
    let mut sheet = Sheet::new(&surface.title, tokens, surface.expression_editor, mtm);
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
            sheet.heading(group.title);
        }
        match field.piece {
            NativePiece::NumberField if field.values.len() == 2 => {
                let (a, b) = (format!("{}.0", field.id), format!("{}.1", field.id));
                sheet.pair(
                    &field.label,
                    [
                        (&a, "Width", &field.values[0]),
                        (&b, "Height", &field.values[1]),
                    ],
                    &field.unit,
                    field.enabled,
                );
            }
            NativePiece::NumberField => {
                sheet.number(&field.id, &field.label, value, &field.unit, field.enabled)
            }
            NativePiece::ColourWell => {
                let well = NSColorWell::initWithFrame(NSColorWell::alloc(mtm), NSRect::ZERO);
                if let Some(rgb) = hex_colour(value) {
                    well.setColor(&crate::dialogs::sheet_layout::native::colour(rgb));
                }
                sheet.swatch(&field.id, &field.label, well);
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
            _ if surface.expression_editor => {
                sheet.editor(&field.id, &field.label, value, EXPRESSION_LINES)
            }
            _ => sheet.text(&field.id, &field.label, value, field.secure, field.enabled),
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
            if surface.expression_editor {
                // Under an expression, on the sheet's text column: what the
                // parser makes of what is typed (two lines kept for it, so
                // the sheet never changes height), then the names an
                // expression can use.
                let (status, bad) = crate::expression_sheet::status(value);
                let label = sheet.line_reserving(
                    &status,
                    "\n",
                    tokens.typography.label.into(),
                    if bad { tokens.colors.danger } else { rgb },
                );
                descriptions.insert(field.id.clone(), label);
                sheet.line(text, tokens.typography.minimum.into(), tokens.colors.muted);
            } else {
                let label = sheet.note(text, &reserve, rgb);
                descriptions.insert(field.id.clone(), label);
            }
        }
        if let Some(error) = field.error {
            sheet.note(error, error, tokens.colors.danger);
        }
    }
    if let Some(warning) = &surface.warning {
        sheet.line(
            warning,
            tokens.typography.minimum.into(),
            tokens.colors.warning,
        );
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
        // Return is a new line in an editor: its sheet applies on ⌘Return.
        if surface.expression_editor
            && button.default
            && let Some(button) = sheet.buttons.get(button.id)
        {
            button.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
        }
        order.push(button.id);
    }
    sheet.footer(&order, None);
    (sheet, descriptions)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hex_seeds_only_what_it_can_read() {
        assert_eq!(hex_colour("#1a1a2e"), Some(0x1a1a2e));
        assert_eq!(hex_colour("#fff"), Some(0xffffff));
        assert_eq!(hex_colour("#FFFFFF00"), None);
        assert_eq!(hex_colour("red"), None);
        assert_eq!(hex_colour(""), None);
    }
}
