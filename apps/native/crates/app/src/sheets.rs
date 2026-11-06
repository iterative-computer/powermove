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
    NSColorSpace, NSColorWell, NSColorWellStyle, NSControl, NSControlTextEditingDelegate,
    NSPopUpButton, NSTextFieldDelegate, NSUserInterfaceItemIdentification, NSWindow,
};
use objc2_foundation::{NSNotification, NSRect, NSString};

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
    pub actions: async_channel::Receiver<String>,
}
impl OpenSheet {
    pub fn present(sheet: Sheet, owner: &NSWindow, first: Option<&str>) -> Self {
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
        Self {
            sheet,
            owner: owner.into(),
            _target: target,
            actions,
        }
    }
    pub fn text(&self, id: &str) -> String {
        self.sheet
            .fields
            .get(id)
            .map(|f| f.stringValue().to_string())
            .unwrap_or_default()
    }
    pub fn number(&self, id: &str) -> f64 {
        self.text(id).trim().parse().unwrap_or(f64::NAN)
    }
    pub fn popup(&self, id: &str) -> Option<usize> {
        let control = self.sheet.controls.get(id)?.clone();
        let popup = control.downcast::<NSPopUpButton>().ok()?;
        usize::try_from(popup.indexOfSelectedItem()).ok()
    }
    pub fn enable(&self, id: &str, enabled: bool) {
        if let Some(button) = self.sheet.buttons.get(id) {
            button.setEnabled(enabled);
        }
    }
    /// The well's colour as `#RRGGBB`, in sRGB.
    pub fn colour(&self, id: &str) -> Option<String> {
        let control = self.sheet.controls.get(id)?.clone();
        let well = control.downcast::<NSColorWell>().ok()?;
        let colour = well
            .color()
            .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        let channel = |v: f64| (v.clamp(0., 1.) * 255.).round() as u8;
        Some(format!(
            "#{:02X}{:02X}{:02X}",
            channel(colour.redComponent()),
            channel(colour.greenComponent()),
            channel(colour.blueComponent())
        ))
    }
    pub fn close(&self) {
        self.owner.endSheet(&self.sheet.window);
        self.sheet.window.orderOut(None);
    }
}
fn hex_colour(hex: &str) -> Option<u32> {
    u32::from_str_radix(hex.strip_prefix('#')?, 16).ok()
}
/// `Surface` forms: labelled controls in spec order and its footer buttons.
/// A multi-value number field makes one well per value: `{id}.0`, `{id}.1`.
pub fn from_surface(surface: &Surface, tokens: &pm_design::Tokens) -> Sheet {
    let mtm = MainThreadMarker::new().unwrap();
    let rows = surface.fields.len() + usize::from(surface.warning.is_some());
    let mut sheet = Sheet::new(&surface.title, rows, tokens, false, mtm);
    for field in &surface.fields {
        let value = field.values.first().map_or("", String::as_str);
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
                sheet.control(&field.id, &field.label, well.into_super());
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
                popup.setEnabled(field.enabled);
                sheet.control(&field.id, &field.label, popup.into_super().into_super());
            }
            _ => sheet.text(&field.id, &field.label, value, false, field.enabled),
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
        order.push(button.id);
    }
    sheet.footer(&order, None);
    sheet
}
