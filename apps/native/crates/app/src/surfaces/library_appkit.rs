//! System NSAlert adapter. Call on the owning window's main executor.
//! LibraryController serializes prompts; closing the alert always cancels.
use super::model::Alert;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSButtonCell, NSModalResponse, NSWindow,
};
use objc2_foundation::NSString;

pub fn present_alert(window: &NSWindow, spec: &Alert) -> async_channel::Receiver<Option<usize>> {
    assert!(spec.default < spec.buttons.len());
    assert_ne!(
        spec.destructive,
        Some(spec.default),
        "destructive actions cannot be default"
    );
    let mtm = MainThreadMarker::new().expect("AppKit main executor");
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&spec.message));
    alert.setInformativeText(&NSString::from_str(&spec.detail));
    if spec.critical {
        alert.setAlertStyle(NSAlertStyle::Critical);
    }
    let mut default_cell = None;
    for (i, label) in spec.buttons.iter().enumerate() {
        let button = alert.addButtonWithTitle(&NSString::from_str(label));
        // Adding the first button installs Return; clear it explicitly before
        // setting Cancel as default for Delete Immediately.
        button.setKeyEquivalent(&NSString::from_str(if *label == "Cancel" {
            "\u{1b}"
        } else if i == spec.default {
            "\r"
        } else {
            ""
        }));
        button.setHasDestructiveAction(spec.destructive == Some(i));
        if i == spec.default {
            default_cell = button
                .cell()
                .and_then(|cell| cell.downcast::<NSButtonCell>().ok());
        }
    }
    // Return uses the default cell while Cancel keeps its Escape equivalent.
    alert.window().setDefaultButtonCell(default_cell.as_deref());
    let count = spec.buttons.len();
    let (tx, rx) = async_channel::bounded(1);
    let done = block2::RcBlock::new(move |response: NSModalResponse| {
        let index = usize::try_from(response - NSAlertFirstButtonReturn)
            .ok()
            .filter(|i| *i < count);
        let _ = tx.try_send(index);
    });
    // No app-modal loop: every library operation has an owner window.
    alert.beginSheetModalForWindow_completionHandler(window, Some(&done));
    rx
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
    for (label, _, _) in PRESETS {
        popup.addItemWithTitle(&NSString::from_str(label));
    }
    popup
        .menu()
        .unwrap()
        .addItem(&NSMenuItem::separatorItem(mtm));
    popup.addItemWithTitle(&NSString::from_str(CUSTOM_PRESET));
    popup.selectItemAtIndex(form.preset.unwrap_or(PRESETS.len() + 1) as isize);
    popup.setEnabled(!form.submitting);
    sheet.control("resolution", "Resolution", popup.into_super().into_super());
    // Two number wells in the one shared value column, with the inspector Link.
    let group = NSStackView::new(mtm);
    group.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    group.setSpacing(tokens.metrics.gap.into());
    for (id, label, value) in [
        ("width", "Width", &form.width),
        ("height", "Height", &form.height),
    ] {
        let field = NSTextField::textFieldWithString(&NSString::from_str(value), mtm);
        field.setAccessibilityLabel(Some(&NSString::from_str(label)));
        field.setEnabled(!form.submitting);
        group.addArrangedSubview(&field);
        sheet.fields.insert(id.into(), field);
    }
    let (symbol, ax, tip) = form.link_presentation();
    sheet.button("link", "", !form.submitting, false, false);
    let link = &sheet.buttons["link"];
    link.setBordered(false);
    link.setButtonType(NSButtonType::PushOnPushOff);
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
    group.addArrangedSubview(link);
    sheet.row("Size", &group);
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
    well.setColorWellStyle(NSColorWellStyle::Minimal);
    well.setEnabled(!form.submitting);
    // Authored strings are never converted here. The colour worker binds the
    // preview colour without changing form.background (N3).
    sheet.control("background", "Background", well.into_super());
    let (label, create, cancel) = form.buttons();
    sheet.button("cancel", "Cancel", cancel, false, true);
    sheet.button("create", label, create, true, false);
    sheet.footer(&["cancel", "create"], None);
    sheet
}
