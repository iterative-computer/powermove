//! AppKit sheets are asynchronous. No modal run loop or UI-thread file IO.
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_app_kit::*;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::path::PathBuf;

pub fn paths(window: &NSWindow, save: bool) -> async_channel::Receiver<Option<PathBuf>> {
    paths_named(window, save, "Project.pmv")
}
pub fn paths_named(
    window: &NSWindow,
    save: bool,
    name: &str,
) -> async_channel::Receiver<Option<PathBuf>> {
    let (tx, rx) = async_channel::bounded(1);
    let mtm = MainThreadMarker::new().unwrap();
    if save {
        let panel = NSSavePanel::savePanel(mtm);
        panel.setNameFieldStringValue(&NSString::from_str(name));
        let retained = panel.clone();
        let done = block2::RcBlock::new(move |result: NSModalResponse| {
            let path = (result == NSModalResponseOK)
                .then(|| retained.URL())
                .flatten()
                .and_then(|u| u.path())
                .map(|p| PathBuf::from(p.to_string()));
            let _ = tx.try_send(path);
        });
        panel.beginSheetModalForWindow_completionHandler(window, &done);
    } else {
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        let retained = panel.clone();
        let done = block2::RcBlock::new(move |result: NSModalResponse| {
            let path = (result == NSModalResponseOK)
                .then(|| retained.URL())
                .flatten()
                .and_then(|u| u.path())
                .map(|p| PathBuf::from(p.to_string()));
            let _ = tx.try_send(path);
        });
        panel.beginSheetModalForWindow_completionHandler(window, &done);
    }
    rx
}

/// Button index, starting at zero. Closing a sheet always chooses cancellation.
pub fn choice(
    window: &NSWindow,
    title: &str,
    detail: &str,
    buttons: &[&str],
) -> async_channel::Receiver<usize> {
    let mtm = MainThreadMarker::new().unwrap();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(detail));
    for b in buttons {
        alert.addButtonWithTitle(&NSString::from_str(b));
    }
    let (tx, rx) = async_channel::bounded(1);
    let done = block2::RcBlock::new(move |r: NSModalResponse| {
        let _ = tx.try_send((r - 1000).max(0) as usize);
    });
    alert.beginSheetModalForWindow_completionHandler(window, Some(&done));
    rx
}

pub fn field(
    window: &NSWindow,
    title: &str,
    value: &str,
) -> async_channel::Receiver<Option<String>> {
    let mtm = MainThreadMarker::new().unwrap();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.addButtonWithTitle(&NSString::from_str("Apply"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(260., 28.)),
    );
    field.setStringValue(&NSString::from_str(value));
    alert.setAccessoryView(Some(&field));
    let (tx, rx) = async_channel::bounded(1);
    let f: Retained<NSTextField> = field.clone();
    let done = block2::RcBlock::new(move |r: NSModalResponse| {
        let _ = tx.try_send((r == NSAlertFirstButtonReturn).then(|| f.stringValue().to_string()));
    });
    alert.beginSheetModalForWindow_completionHandler(window, Some(&done));
    unsafe {
        field.selectText(None);
    }
    rx
}

pub fn history_labels(undo: Option<&str>, redo: Option<&str>) {
    let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
    let Some(root) = app.mainMenu() else { return };
    for top in root.itemArray() {
        if top.title().to_string() != "Edit" {
            continue;
        }
        if let Some(menu) = top.submenu() {
            for (index, (verb, label)) in [("Undo", undo), ("Redo", redo)].into_iter().enumerate() {
                if let Some(item) = menu.itemAtIndex(index as isize) {
                    item.setTitle(&NSString::from_str(
                        &label.map_or(verb.to_string(), |l| format!("{verb} {l}")),
                    ));
                    item.setEnabled(label.is_some());
                }
            }
        }
    }
}
