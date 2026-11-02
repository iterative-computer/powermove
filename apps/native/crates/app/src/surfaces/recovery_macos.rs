//! System NSAlert presenter. Host invokes this on main after dequeuing a Show.
//! AppKit owns icon, system alert semantics, geometry and sheet placement.
use super::recovery::{Action, Key, Owner, Presentation, Prompt, RecoveryPlatform};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSEventModifierFlags, NSModalResponse, NSWindow,
};
use objc2_foundation::NSString;

#[derive(Clone, Debug)]
pub struct Response {
    pub id: String,
    pub revision: u64,
    pub action: Option<Action>,
}
/// Window ownership must be resolved by the host before calling. A missing
/// document window is a stale target, not permission to show app-modal UI.
pub fn sheet(prompt: &Prompt, window: &NSWindow) -> async_channel::Receiver<Response> {
    let alert = make_alert(prompt);
    let (tx, rx) = async_channel::bounded(1);
    let prompt = prompt.clone();
    let retained_alert = alert.clone();
    let done = block2::RcBlock::new(move |r: NSModalResponse| {
        let _keep_alive = &retained_alert;
        let action = usize::try_from(r - NSAlertFirstButtonReturn)
            .ok()
            .and_then(|index| prompt.alert.buttons.get(index))
            .map(|b| b.action);
        let _ = tx.try_send(Response {
            id: prompt.id.clone(),
            revision: prompt.revision,
            action,
        });
    });
    alert.beginSheetModalForWindow_completionHandler(window, Some(&done));
    rx
}
fn make_alert(prompt: &Prompt) -> Retained<NSAlert> {
    let mtm = MainThreadMarker::new().expect("NSAlert must be presented on main");
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&prompt.alert.message));
    alert.setInformativeText(&NSString::from_str(&prompt.alert.informative));
    for b in &prompt.alert.buttons {
        let button = alert.addButtonWithTitle(&NSString::from_str(b.label));
        let key = if b.default {
            "\r"
        } else if b.escape {
            "\u{1b}"
        } else {
            ""
        };
        button.setKeyEquivalent(&NSString::from_str(key));
        button.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
        button.setHasDestructiveAction(b.destructive);
    }
    debug_assert_eq!(
        prompt.alert.key(Key::Return),
        prompt.alert.buttons.first().map(|b| b.action)
    );
    alert
}
/// The shell calls this only for an Application owner after checking that no
/// window owns the action. runModal runs AppKit's modal loop, never JavaScript.
pub fn application(prompt: &Prompt) -> Option<Response> {
    if prompt.owner != Owner::Application {
        return None;
    }
    let result = make_alert(prompt).runModal();
    let action = usize::try_from(result - NSAlertFirstButtonReturn)
        .ok()
        .and_then(|i| prompt.alert.buttons.get(i))
        .map(|b| b.action);
    Some(Response {
        id: prompt.id.clone(),
        revision: prompt.revision,
        action,
    })
}
/// Worker-to-main AppKit delivery. The shell drains this channel, resolves the
/// exact Window owner and calls sheet/application; a lost window is dismissed,
/// never converted into Application ownership. Dismiss ends the retained sheet.
pub struct AppKitPlatform {
    sender: async_channel::Sender<Presentation>,
}
impl AppKitPlatform {
    pub fn channel(capacity: usize) -> (Self, async_channel::Receiver<Presentation>) {
        let (sender, receiver) = async_channel::bounded(capacity.max(1));
        (Self { sender }, receiver)
    }
}
impl RecoveryPlatform for AppKitPlatform {
    type Error = async_channel::TrySendError<Presentation>;
    fn enqueue(&mut self, presentation: Presentation) -> Result<(), Self::Error> {
        self.sender.try_send(presentation)
    }
}

/// Call once per accepted notice/panel-state change, never from a paint callback.
pub fn announce(
    _main_thread: MainThreadMarker,
    window: &NSWindow,
    text: &str,
    priority: super::recovery::notices::Priority,
) {
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSAccessibilityAnnouncementKey, NSAccessibilityAnnouncementRequestedNotification,
        NSAccessibilityPostNotificationWithUserInfo, NSAccessibilityPriorityKey,
        NSAccessibilityPriorityLevel,
    };
    use objc2_foundation::{NSDictionary, NSNumber};
    let text = NSString::from_str(text);
    let priority = NSNumber::new_isize(match priority {
        super::recovery::notices::Priority::Medium => NSAccessibilityPriorityLevel::Medium.0,
        super::recovery::notices::Priority::High => NSAccessibilityPriorityLevel::High.0,
    });
    let values: [&AnyObject; 2] = [&text, &priority];
    // SAFETY: Keys are Apple's NSString notification keys; values have the
    // documented NSString/NSNumber types. NSWindow is an accessibility element.
    unsafe {
        let info = NSDictionary::from_slices(
            &[NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey],
            &values,
        );
        NSAccessibilityPostNotificationWithUserInfo(
            window,
            NSAccessibilityAnnouncementRequestedNotification,
            Some(&info),
        );
    }
}
