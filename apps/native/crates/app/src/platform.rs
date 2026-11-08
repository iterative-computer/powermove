//! AppKit sheets are asynchronous. No modal run loop or UI-thread file IO.
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_app_kit::*;
use objc2_foundation::NSString;
use std::path::PathBuf;

/// An open sheet that takes `.cube` files.
pub fn cube_file(window: &NSWindow) -> async_channel::Receiver<Option<PathBuf>> {
    let (tx, rx) = async_channel::bounded(1);
    let mtm = MainThreadMarker::new().unwrap();
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setMessage(Some(&NSString::from_str("Choose a .cube LUT")));
    #[allow(deprecated)]
    panel.setAllowedFileTypes(Some(&objc2_foundation::NSArray::from_retained_slice(&[
        NSString::from_str("cube"),
    ])));
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
    rx
}

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

/// Hands a web link to the default browser.
pub fn open_url(url: &str) -> bool {
    objc2_foundation::NSURL::URLWithString(&NSString::from_str(url))
        .is_some_and(|url| objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&url))
}

/// Import Media: several items at once, filtered to `extensions` when it has any.
pub fn import_paths(
    window: &NSWindow,
    folders: bool,
    multiple: bool,
    extensions: &[&str],
) -> async_channel::Receiver<Vec<PathBuf>> {
    choose_paths(window, folders, multiple, extensions, "Import")
}
/// Files for the Agent panel's Add Attachments.
pub fn attach_paths(window: &NSWindow) -> async_channel::Receiver<Vec<PathBuf>> {
    choose_paths(window, false, true, &[], "Attach")
}
fn choose_paths(
    window: &NSWindow,
    folders: bool,
    multiple: bool,
    extensions: &[&str],
    prompt: &str,
) -> async_channel::Receiver<Vec<PathBuf>> {
    use objc2::{msg_send, runtime::AnyObject};
    use objc2_foundation::NSArray;
    let (tx, rx) = async_channel::bounded(1);
    let mtm = MainThreadMarker::new().unwrap();
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(!folders);
    panel.setCanChooseDirectories(folders);
    panel.setAllowsMultipleSelection(multiple);
    panel.setPrompt(Some(&NSString::from_str(prompt)));
    let types: Vec<Retained<AnyObject>> = extensions
        .iter()
        .filter_map(|extension| unsafe {
            let extension = NSString::from_str(extension);
            msg_send![objc2::class!(UTType), typeWithFilenameExtension: &*extension]
        })
        .collect();
    if !types.is_empty() {
        let types = NSArray::from_retained_slice(&types);
        unsafe {
            let _: () = msg_send![&*panel, setAllowedContentTypes: &*types];
        }
    }
    let retained = panel.clone();
    let done = block2::RcBlock::new(move |result: NSModalResponse| {
        let paths = if result == NSModalResponseOK {
            retained
                .URLs()
                .iter()
                .filter_map(|url| url.path())
                .map(|path| PathBuf::from(path.to_string()))
                .collect()
        } else {
            vec![]
        };
        let _ = tx.try_send(paths);
    });
    panel.beginSheetModalForWindow_completionHandler(window, &done);
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
    for (index, b) in buttons.iter().enumerate() {
        let button = alert.addButtonWithTitle(&NSString::from_str(b));
        if *b == "Cancel" {
            button.setKeyEquivalent(&NSString::from_str("\u{1b}"));
        } else if index == 0 {
            button.setKeyEquivalent(&NSString::from_str("\r"));
        }
        if *b == "Don’t Save" {
            button.setKeyEquivalent(&NSString::from_str("d"));
            button.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
        }
    }
    let (tx, rx) = async_channel::bounded(1);
    let done = block2::RcBlock::new(move |r: NSModalResponse| {
        let _ = tx.try_send(choice_index(r));
    });
    alert.beginSheetModalForWindow_completionHandler(window, Some(&done));
    rx
}

/// Selects `path` in Finder.
pub fn reveal(path: &std::path::Path) {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSArray, NSURL};
    if let Some(url) = NSURL::from_file_path(path) {
        NSWorkspace::sharedWorkspace()
            .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
    }
}
/// A one-button alert as a sheet on the key window.
pub fn note(title: &str, detail: &str) {
    let mtm = MainThreadMarker::new().unwrap();
    let Some(window) = objc2_app_kit::NSApplication::sharedApplication(mtm).keyWindow() else {
        return;
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(detail));
    alert.addButtonWithTitle(&NSString::from_str("OK"));
    alert.beginSheetModalForWindow_completionHandler(&window, None);
}

fn choice_index(response: NSModalResponse) -> usize {
    usize::try_from(response - NSAlertFirstButtonReturn).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alert_buttons_keep_their_order_and_aborted_sheets_never_choose_save_or_recover() {
        assert_eq!(choice_index(NSAlertFirstButtonReturn), 0);
        assert_eq!(choice_index(NSAlertSecondButtonReturn), 1);
        assert_eq!(choice_index(NSAlertThirdButtonReturn), 2);
        assert_eq!(choice_index(NSModalResponseAbort), usize::MAX);
        assert_eq!(choice_index(NSModalResponseCancel), usize::MAX);
    }
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

// Preserve GPUI's existing application delegate while intercepting termination
// before its non-cancellable shutdown callback. All other selectors forward.
use objc2::{
    DefinedClass, define_class, msg_send,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel},
};
use std::cell::Cell;
pub struct QuitState {
    previous: Retained<ProtocolObject<dyn NSApplicationDelegate>>,
    sender: async_channel::Sender<()>,
    allowed: Cell<bool>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMNativeQuitDelegate"]
    #[ivars = QuitState]
    pub struct QuitDelegate;
    unsafe impl NSObjectProtocol for QuitDelegate {
        #[unsafe(method(respondsToSelector:))]
        fn responds_to_selector(&self, selector: Sel) -> bool {
            selector == objc2::sel!(applicationShouldTerminate:)
                || unsafe { msg_send![&*self.ivars().previous, respondsToSelector: selector] }
        }
    }
    unsafe impl NSApplicationDelegate for QuitDelegate {
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _: &NSApplication) -> NSApplicationTerminateReply {
            if self.ivars().allowed.get() { NSApplicationTerminateReply::TerminateNow }
            else { let _=self.ivars().sender.try_send(()); NSApplicationTerminateReply::TerminateCancel }
        }
    }
    impl QuitDelegate {
        #[unsafe(method(forwardingTargetForSelector:))]
        fn forward(&self, _: Sel) -> *const AnyObject {
            &*self.ivars().previous as *const _ as *const AnyObject
        }
    }
);
pub fn quit_guard() -> (Retained<QuitDelegate>, async_channel::Receiver<()>) {
    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let (sender, rx) = async_channel::bounded(1);
    let previous = app.delegate().expect("GPUI application delegate");
    let this = QuitDelegate::alloc(mtm).set_ivars(QuitState {
        previous,
        sender,
        allowed: Cell::new(false),
    });
    let guard: Retained<QuitDelegate> = unsafe { objc2::msg_send![super(this), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*guard)));
    (guard, rx)
}
impl QuitDelegate {
    pub fn allow(&self) {
        self.ivars().allowed.set(true);
    }
}

/// PNG sequences choose a parent directory; publication owns a fresh child
/// folder, so existing frames are never overwritten.
pub fn export_destination(
    window: &NSWindow,
    folder: bool,
    name: &str,
) -> async_channel::Receiver<Option<PathBuf>> {
    if !folder {
        return paths_named(window, true, name);
    }
    let (tx, rx) = async_channel::bounded(1);
    let mtm = MainThreadMarker::new().unwrap();
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setCanCreateDirectories(true);
    let retained = panel.clone();
    // The suggestion is authored by the Export model, never a pathname.
    let name = PathBuf::from(name).file_name().map(|s| s.to_owned());
    let done = block2::RcBlock::new(move |result: NSModalResponse| {
        let path = (result == NSModalResponseOK)
            .then(|| retained.URL())
            .flatten()
            .and_then(|u| u.path())
            .map(|p| PathBuf::from(p.to_string()));
        let path = path
            .zip(name.clone())
            .map(|(parent, name)| parent.join(name));
        let _ = tx.try_send(path);
    });
    panel.beginSheetModalForWindow_completionHandler(window, &done);
    rx
}

/// The AppKit window behind a GPUI window (a panel's own window, for sheets).
pub fn ns_window(window: &gpui::Window) -> Option<Retained<NSWindow>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    let view: Retained<objc2_app_kit::NSView> =
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast())? };
    view.window()
}
