//! System-only NSAlert / NSOpenPanel / NSSavePanel adapter.
//! Construct and drive on the main thread. Completion is sent to the host's
//! event queue; no callbacks enter JavaScript or wait for a kernel worker.
use super::*;
use objc2::{MainThreadMarker, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::*;
use objc2_foundation::{NSArray, NSString};
use std::sync::mpsc::Sender;

pub struct AppKitHost {
    windows: BTreeMap<WindowId, Retained<NSWindow>>,
    active: BTreeMap<Ticket, Retained<NSWindow>>,
    output: Sender<(Ticket, Response)>,
}
impl AppKitHost {
    pub fn new(output: Sender<(Ticket, Response)>) -> Self {
        Self {
            windows: BTreeMap::new(),
            active: BTreeMap::new(),
            output,
        }
    }
    pub fn attach(&mut self, id: WindowId, window: Retained<NSWindow>) {
        self.windows.insert(id, window);
    }
    pub fn detach(&mut self, id: WindowId) {
        self.windows.remove(&id);
    }
    pub fn completed(&mut self, ticket: Ticket) {
        self.active.remove(&ticket);
    }
    fn owner(&self, target: &Target) -> Option<&NSWindow> {
        match target.owner {
            Owner::Application => None,
            Owner::Window(id) => self.windows.get(&id).map(|w| &**w),
        }
    }
    fn alert(&mut self, request: &Request, description: &Alert, mtm: MainThreadMarker) {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(&description.message));
        alert.setInformativeText(&NSString::from_str(&description.informative));
        // All document alerts use the system app icon and informational style;
        // none is specified critical. No suppression or custom accessory.
        alert.setAlertStyle(NSAlertStyle::Informational);
        for button in &description.buttons {
            let native = alert.addButtonWithTitle(&NSString::from_str(button.label));
            native.setKeyEquivalent(&NSString::from_str(match button.key {
                Some(Key::Return) => "\r",
                Some(Key::Escape) => "\u{1b}",
                Some(Key::CommandD) => "d",
                None => "",
            }));
            native.setKeyEquivalentModifierMask(if button.key == Some(Key::CommandD) {
                NSEventModifierFlags::Command
            } else {
                NSEventModifierFlags::empty()
            });
            native.setHasDestructiveAction(button.destructive);
        }
        let ticket = request.ticket;
        let output = self.output.clone();
        let description = description.clone();
        let finish = move |result: NSModalResponse| {
            let index = usize::try_from(result - NSAlertFirstButtonReturn).ok();
            let _ = output.send((ticket, Response::Alert(description.action_at(index))));
        };
        self.active.insert(ticket, alert.window());
        if let Some(owner) = self.owner(&request.target) {
            alert.beginSheetModalForWindow_completionHandler(
                owner,
                Some(&block2::RcBlock::new(finish)),
            );
        } else {
            finish(alert.runModal());
        }
    }
    fn picker(&mut self, request: &Request, picker: &Picker, mtm: MainThreadMarker) {
        if let PickerKind::Save { name } = &picker.kind {
            let panel = NSSavePanel::savePanel(mtm);
            panel.setTitle(Some(&NSString::from_str(picker.title)));
            panel.setPrompt(Some(&NSString::from_str(picker.prompt)));
            panel.setNameFieldStringValue(&NSString::from_str(name));
            panel.setCanCreateDirectories(true);
            panel.setAllowsOtherFileTypes(false);
            // Do not override extensionHidden: the system remembers the user's
            // preference. Content type enforces .pmv and owns overwrite checking.
            set_content_types(&panel, &["pmv".into()], true);
            let retained = panel.clone();
            let output = self.output.clone();
            let ticket = request.ticket;
            let finish = move |response: NSModalResponse| {
                let paths = if response == NSModalResponseOK {
                    retained
                        .URL()
                        .and_then(|u| u.path())
                        .map(|p| PathBuf::from(p.to_string()))
                        .into_iter()
                        .collect()
                } else {
                    vec![]
                };
                let _ = output.send((ticket, Response::Paths(paths)));
            };
            self.active.insert(ticket, panel.clone().into_super());
            if let Some(owner) = self.owner(&request.target) {
                panel.beginSheetModalForWindow_completionHandler(
                    owner,
                    &block2::RcBlock::new(finish),
                );
            } else {
                finish(panel.runModal());
            }
        } else {
            let panel = NSOpenPanel::openPanel(mtm);
            panel.setTitle(Some(&NSString::from_str(picker.title)));
            panel.setPrompt(Some(&NSString::from_str(picker.prompt)));
            panel.setCanChooseFiles(picker.files);
            panel.setCanChooseDirectories(picker.folders);
            panel.setAllowsMultipleSelection(picker.multiple);
            panel.setCanCreateDirectories(picker.create_directories);
            match &picker.kind {
                PickerKind::Open => set_content_types(&panel, &["pmv".into()], true),
                PickerKind::Import { supported_types } => {
                    set_content_types(&panel, supported_types, false)
                }
                _ => {}
            }
            let retained = panel.clone();
            let output = self.output.clone();
            let ticket = request.ticket;
            let finish = move |response: NSModalResponse| {
                let paths = if response == NSModalResponseOK {
                    retained
                        .URLs()
                        .iter()
                        .filter_map(|u| u.path())
                        .map(|p| PathBuf::from(p.to_string()))
                        .collect()
                } else {
                    vec![]
                };
                let _ = output.send((ticket, Response::Paths(paths)));
            };
            self.active
                .insert(ticket, panel.clone().into_super().into_super());
            if let Some(owner) = self.owner(&request.target) {
                panel.beginSheetModalForWindow_completionHandler(
                    owner,
                    &block2::RcBlock::new(finish),
                );
            } else {
                finish(panel.runModal());
            }
        }
    }
}
impl DocumentHost for AppKitHost {
    fn present(&mut self, request: Request) {
        let mtm =
            MainThreadMarker::new().expect("AppKit document surfaces require the main thread");
        if matches!(request.target.owner,Owner::Window(id) if !self.windows.contains_key(&id)) {
            let _ = self.output.send((request.ticket, Response::Cancel));
            return;
        }
        match &request.presentation {
            Presentation::Alert(alert) => self.alert(&request, alert, mtm),
            Presentation::Picker(picker) => self.picker(&request, picker, mtm),
            Presentation::Finishing(_) => {
                // DESIGN-TODO(opus): design/surfaces/README §2 specifies
                // m.sheet_inset and m.sheet_value_column, neither exists in
                // pm-design. Keep an inspectable neutral placeholder; do not
                // substitute made-up geometry or a differently styled alert.
                let _ = self.output.send((
                    request.ticket,
                    Response::Unavailable(
                        "DESIGN-TODO(opus): missing sheet_inset / sheet_value_column tokens",
                    ),
                ));
            }
        }
    }
    fn dismiss(&mut self, ticket: Ticket) {
        if let Some(window) = self.active.remove(&ticket) {
            if let Some(parent) = window.sheetParent() {
                parent.endSheet_returnCode(&window, NSModalResponseCancel);
            } else {
                NSApplication::sharedApplication(MainThreadMarker::new().unwrap()).abortModal();
                window.orderOut(None);
            }
        }
    }
}
/// Use the real UniformTypeIdentifiers class without adding an unowned app
/// manifest dependency. The resulting objects are retained in NSArray and
/// passed to AppKit's typed content-type API (not legacy filename filters).
fn set_content_types(panel: &NSSavePanel, values: &[String], extensions: bool) {
    let types: Vec<Retained<AnyObject>> = values
        .iter()
        .filter_map(|value| unsafe {
            let value = NSString::from_str(value);
            if extensions {
                msg_send![objc2::class!(UTType),typeWithFilenameExtension:&*value]
            } else {
                msg_send![objc2::class!(UTType),typeWithIdentifier:&*value]
            }
        })
        .collect();
    let types = NSArray::from_retained_slice(&types);
    unsafe {
        let _: () = msg_send![panel,setAllowedContentTypes:&*types];
    }
}
