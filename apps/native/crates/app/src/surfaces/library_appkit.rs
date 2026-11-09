//! The New Project sheet's AppKit controls.
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_foundation::NSString;

/// The aspect Link's symbol, label, tip and state for the form's current lock.
pub fn show_link(link: &objc2_app_kit::NSButton, form: &super::form::NewProjectForm) {
    use objc2_app_kit::{NSAccessibility, NSControlStateValueOff, NSControlStateValueOn, NSImage};
    let (symbol, ax, tip) = form.link_presentation();
    link.setAccessibilityLabel(Some(&NSString::from_str(ax)));
    link.setToolTip(Some(&NSString::from_str(tip)));
    link.setState(if form.linked == Some(true) {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
    link.setImage(
        NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol),
            None,
        )
        .as_deref(),
    );
}

/// S018 native fields; publication/validation remain NewProjectForm-owned.
/// The caller binds the control IDs to its main-thread input queue, and uses the
/// kit ColourPopover for the minimal native colour well's action.
pub fn new_project_sheet(
    form: &super::form::NewProjectForm,
    tokens: &pm_design::Tokens,
    mtm: MainThreadMarker,
) -> super::sheet_layout::native::Sheet {
    use super::{form::*, sheet_layout::native::Sheet};
    use objc2_app_kit::*;
    let errors = form.validation();
    let mut sheet = Sheet::new(
        SHEET_TITLE,
        FORM_ROWS.len()
            + [errors.name, errors.size, errors.fps, errors.duration]
                .iter()
                .filter(|e| e.is_some())
                .count(),
        tokens,
        false,
        mtm,
    );
    sheet.text("name", "Name", &form.name, false, !form.submitting);
    if let Some(error) = errors.name {
        sheet.line(
            error,
            tokens.typography.minimum.into(),
            tokens.colors.danger,
        );
    }
    let popup = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        objc2_foundation::NSRect::ZERO,
        false,
    );
    for (label, _, _) in pm_builtins::RESOLUTIONS {
        popup.addItemWithTitle(&NSString::from_str(label));
    }
    popup
        .menu()
        .unwrap()
        .addItem(&NSMenuItem::separatorItem(mtm));
    popup.addItemWithTitle(&NSString::from_str(CUSTOM_PRESET));
    popup.selectItemAtIndex(form.preset.unwrap_or(pm_builtins::RESOLUTIONS.len() + 1) as isize);
    popup.setEnabled(!form.submitting);
    sheet.control("resolution", "Resolution", popup.into_super().into_super());
    // `1920 × 1080`, with the inspector Link in the unit column.
    sheet.pair(
        "Size",
        [
            ("width", "Width", &form.width),
            ("height", "Height", &form.height),
        ],
        "",
        !form.submitting,
    );
    for id in ["width", "height"] {
        let field = &sheet.fields[id];
        // A size reports when its editing ends, which applies the aspect link
        // to a whole number rather than to every keystroke.
        field.setContinuous(false);
        // A cell property: NSTextField itself doesn't answer it (sending it
        // there aborted the app on File ▸ New Project).
        if let Some(cell) = field.cell() {
            unsafe {
                let _: () = objc2::msg_send![&*cell, setSendsActionOnEndEditing: true];
            }
        }
    }
    sheet.button("link", "", !form.submitting, false, false);
    let link = sheet.buttons["link"].clone();
    link.setBordered(false);
    link.setButtonType(NSButtonType::PushOnPushOff);
    show_link(&link, form);
    sheet.trailing(&link);
    if let Some(error) = errors.size {
        sheet.line(
            error,
            tokens.typography.minimum.into(),
            tokens.colors.danger,
        );
    }

    sheet.number("fps", "Frame Rate", &form.fps, "fps", !form.submitting);
    if let Some(error) = errors.fps {
        sheet.line(
            error,
            tokens.typography.minimum.into(),
            tokens.colors.danger,
        );
    }
    sheet.number(
        "duration",
        "Duration",
        &form.duration,
        "s",
        !form.submitting,
    );
    if let Some(error) = errors.duration {
        sheet.line(
            error,
            tokens.typography.minimum.into(),
            tokens.colors.danger,
        );
    }
    let well = NSColorWell::initWithFrame(NSColorWell::alloc(mtm), objc2_foundation::NSRect::ZERO);
    well.setEnabled(!form.submitting);
    // A preview only. The authored string is read back once the well changes (N3).
    if let Some(rgb) = crate::sheets::hex_colour(&form.background) {
        well.setColor(&crate::dialogs::sheet_layout::native::colour(rgb));
    }
    sheet.swatch("background", "Background", well);
    let (label, create, cancel) = form.buttons();
    sheet.button("cancel", "Cancel", cancel, false, true);
    sheet.button("create", label, create, true, false);
    sheet.footer(&["cancel", "create"], None);
    sheet
}
