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
/// Options depth first: a choice's index counts every row before it, so a flat
/// list answers its own positions and a nested one indexes this list.
pub fn flatten(options: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(options.len());
    for option in options {
        out.push(option.clone());
        if let Some(children) = option.get("children").and_then(Value::as_array) {
            out.extend(flatten(children));
        }
    }
    out
}
fn build(
    options: &[Value],
    next: &mut isize,
    target: &MenuTarget,
    mtm: MainThreadMarker,
) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for option in options {
        let tag = *next;
        *next += 1;
        if option.is_null() || option.get("separator") == Some(&Value::Bool(true)) {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        }
        let label = option
            .get("label")
            .and_then(Value::as_str)
            .or_else(|| option.as_str())
            .unwrap_or("");
        let children = option.get("children").and_then(Value::as_array);
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(label),
                children.is_none().then_some(sel!(choose:)),
                &NSString::new(),
            )
        };
        item.setTag(tag);
        item.setEnabled(
            option.get("disabled") != Some(&Value::Bool(true))
                && option.get("enabled") != Some(&Value::Bool(false)),
        );
        match children {
            Some(children) => {
                let submenu = build(children, next, target, mtm);
                item.setSubmenu(Some(&submenu));
            }
            None => unsafe { item.setTarget(Some(target)) },
        }
        menu.addItem(&item);
    }
    menu
}
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
    let mut next = 0;
    let menu = build(options, &mut next, &target, mtm);
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
