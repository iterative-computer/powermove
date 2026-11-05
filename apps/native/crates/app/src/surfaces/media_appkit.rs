//! Standard NSAlert adapter. The app pumps callbacks asynchronously; it must
//! route presentation through AlertQueue and close it on document retirement.
use super::media::{Action, Alert, Command, MediaHost, Target};
use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSControlStateValueOff, NSControlStateValueOn,
    NSModalResponse, NSModalResponseOK, NSOpenPanel, NSWindow,
};
use objc2_foundation::NSString;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Response {
    pub target: Target,
    /// Aborted/closed system sheets never choose the default action.
    pub action: Option<Action>,
    pub suppression_checked: bool,
}
pub struct AppKitHost {
    window: Retained<NSWindow>,
    window_id: String,
    active: BTreeMap<String, (Target, Retained<NSAlert>)>,
    commands: async_channel::Sender<Command>,
    responses: async_channel::Sender<Response>,
}
#[derive(Clone, Debug)]
pub struct FileChoice {
    pub target: Target,
    pub path: Option<std::path::PathBuf>,
}
impl AppKitHost {
    /// Native relink/replace picker. Path probing stays on the media worker.
    pub fn locate(&self, target: Target) -> async_channel::Receiver<FileChoice> {
        let (tx, rx) = async_channel::bounded(1);
        if target.owner.window != self.window_id {
            return rx;
        }
        let mtm = MainThreadMarker::new().expect("AppKit picker on main thread");
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        let retained = panel.clone();
        let callback = block2::RcBlock::new(move |response: NSModalResponse| {
            let path = (response == NSModalResponseOK)
                .then(|| retained.URL())
                .flatten()
                .and_then(|url| url.path())
                .map(|path| std::path::PathBuf::from(path.to_string()));
            let _ = tx.try_send(FileChoice {
                target: target.clone(),
                path,
            });
        });
        panel.beginSheetModalForWindow_completionHandler(&self.window, &callback);
        rx
    }

    /// Channels must be unbounded: UI enqueue may never block or discard work
    /// because a worker is busy. Disconnection means the owning document retired.
    pub fn new(
        window: Retained<NSWindow>,
        window_id: String,
    ) -> (
        Self,
        async_channel::Receiver<Command>,
        async_channel::Receiver<Response>,
    ) {
        let (commands, command_rx) = async_channel::unbounded();
        let (responses, response_rx) = async_channel::unbounded();
        (
            Self {
                window,
                window_id,
                active: BTreeMap::new(),
                commands,
                responses,
            },
            command_rx,
            response_rx,
        )
    }
}
impl MediaHost for AppKitHost {
    fn enqueue(&mut self, command: Command) {
        if command.target.owner.window == self.window_id {
            let _ = self.commands.try_send(command);
        }
    }
    fn present_alert(&mut self, description: Alert) {
        if description.target.owner.window != self.window_id || !self.active.is_empty() {
            return;
        }
        let mtm = MainThreadMarker::new().expect("AppKit media presentation on main thread");
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(&description.message));
        alert.setInformativeText(&NSString::from_str(&description.informative));
        for (index, action) in description.buttons.iter().enumerate() {
            let button = alert.addButtonWithTitle(&NSString::from_str(action.label()));
            if *action == Action::Cancel {
                button.setKeyEquivalent(&NSString::from_str("\u{1b}"));
            } else if index == 0 {
                button.setKeyEquivalent(&NSString::from_str("\r"));
            }
        }
        if let Some(label) = description.suppression {
            alert.setShowsSuppressionButton(true);
            if let Some(button) = alert.suppressionButton() {
                button.setTitle(&NSString::from_str(label));
                button.setState(NSControlStateValueOff);
            }
        }
        let target = description.target.clone();
        let retained = alert.clone();
        let tx = self.responses.clone();
        let callback = block2::RcBlock::new(move |response: NSModalResponse| {
            let action = usize::try_from(response - NSAlertFirstButtonReturn)
                .ok()
                .and_then(|i| description.buttons.get(i).copied());
            let suppression_checked = retained
                .suppressionButton()
                .is_some_and(|b| b.state() == NSControlStateValueOn);
            let _ = tx.try_send(Response {
                target: description.target.clone(),
                action,
                suppression_checked,
            });
        });
        self.active
            .insert(target.id.clone(), (target, alert.clone()));
        alert.beginSheetModalForWindow_completionHandler(&self.window, Some(&callback));
    }
    fn close_alert(&mut self, target: &Target) {
        if self
            .active
            .get(&target.id)
            .is_some_and(|(active, _)| active == target)
            && let Some((_, alert)) = self.active.remove(&target.id)
            && self.window.attachedSheet().as_deref() == Some(&*alert.window())
        {
            self.window.endSheet(&alert.window());
        }
    }
}
