//! System NSAlert adapter. Call on the owning window's main executor.
//! LibraryController serializes prompts; closing the alert always cancels.
use super::model::Alert;
use objc2::MainThreadMarker;
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
