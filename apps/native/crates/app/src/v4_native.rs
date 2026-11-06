//! System menus for native surface controls. Tracking runs on a deferred main
//! turn; its result is enqueued back to the worker, never a synchronous JS call.
use gpui::{App, Window};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::AnyObject, sel,
};
use objc2_app_kit::{NSMenu, NSMenuItem, NSView};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use serde_json::Value;
use std::cell::Cell;
#[derive(Default)]
struct MenuState {
    selected: Cell<Option<usize>>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[name="PMV4MenuTarget"]
    #[ivars=MenuState]
    struct MenuTarget;
    unsafe impl NSObjectProtocol for MenuTarget {}
    impl MenuTarget{
        #[unsafe(method(choose:))]
        fn choose(&self,sender:&NSMenuItem){self.ivars().selected.set(Some(sender.tag() as usize));}
    }
);
pub fn menu(
    window: &mut Window,
    options: &[Value],
    cx: &mut App,
    choose: impl FnOnce(Option<usize>, &mut App) + 'static,
) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).unwrap().as_raw()
    else {
        return;
    };
    let view: Retained<NSView> =
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
    let mtm = MainThreadMarker::new().unwrap();
    let target: Retained<MenuTarget> = unsafe {
        msg_send![
            super(MenuTarget::alloc(mtm).set_ivars(MenuState::default())),
            init
        ]
    };
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for (index, option) in options.iter().enumerate() {
        if option.is_null() || option.get("separator") == Some(&Value::Bool(true)) {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        }
        let label = option
            .get("label")
            .and_then(Value::as_str)
            .or_else(|| option.as_str())
            .unwrap_or("");
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(label),
                Some(sel!(choose:)),
                &NSString::new(),
            )
        };
        item.setTag(index as isize);
        if option.get("checked") == Some(&Value::Bool(true)) {
            item.setState(objc2_app_kit::NSControlStateValueOn);
        }
        if let Some(tooltip) = option.get("tooltip").and_then(Value::as_str) {
            item.setToolTip(Some(&NSString::from_str(tooltip)));
        }
        item.setEnabled(
            option.get("disabled") != Some(&Value::Bool(true))
                && option.get("enabled") != Some(&Value::Bool(false)),
        );
        unsafe {
            item.setTarget(Some(&target));
        }
        menu.addItem(&item);
    }
    let point = view.convertPoint_fromView(
        view.window()
            .map(|w| w.mouseLocationOutsideOfEventStream())
            .unwrap_or_default(),
        None,
    );
    cx.defer(move |cx| {
        menu.popUpMenuPositioningItem_atLocation_inView(None, point, Some(&view));
        choose(target.ivars().selected.get(), cx);
    });
}

pub fn announce(window: &objc2_app_kit::NSWindow, message: &str) {
    use objc2_app_kit::{
        NSAccessibilityAnnouncementKey, NSAccessibilityAnnouncementRequestedNotification,
        NSAccessibilityPostNotificationWithUserInfo, NSAccessibilityPriorityKey,
        NSAccessibilityPriorityLevel,
    };
    use objc2_foundation::{NSDictionary, NSNumber};
    let text = NSString::from_str(message);
    let priority = NSNumber::new_isize(NSAccessibilityPriorityLevel::Medium.0);
    let values: [&AnyObject; 2] = [text.as_ref(), priority.as_ref()];
    unsafe {
        let info = NSDictionary::from_slices(
            &[NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey],
            &values,
        );
        NSAccessibilityPostNotificationWithUserInfo(
            window.as_ref(),
            NSAccessibilityAnnouncementRequestedNotification,
            Some(&info),
        );
    }
}
