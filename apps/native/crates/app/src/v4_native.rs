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
    // An agent's panel action never leaves a menu on screen: its item is
    // chosen by label, or the menu closes and the agent gets its items.
    if let Some(ns_window) = view.window()
        && let Some(choice) =
            pm_shell::agent_menu::claim(&pm_shell::agent_menu_windows(&ns_window), options)
    {
        // After the caller's update returns, as a chosen row arrives.
        cx.defer(move |cx| choose(choice, cx));
        return;
    }
    let mtm = MainThreadMarker::new().unwrap();
    let target: Retained<MenuTarget> = unsafe {
        msg_send![
            super(MenuTarget::alloc(mtm).set_ivars(MenuState::default())),
            init
        ]
    };
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    append_menu(&menu, options, &target, mtm, &mut 0);
    let point = view.convertPoint_fromView(
        view.window()
            .map(|w| w.mouseLocationOutsideOfEventStream())
            .unwrap_or_default(),
        None,
    );
    // Menu tracking runs a nested run loop that keeps draining the main queue.
    // A deferred effect would hold the app borrowed through it, and the first
    // task that touches the app would panic; a task holds no borrow.
    cx.spawn(async move |cx| {
        menu.popUpMenuPositioningItem_atLocation_inView(None, point, Some(&view));
        let selected = target.ivars().selected.get();
        cx.update(|cx| choose(selected, cx));
    })
    .detach();
}

/// Preorder identity keeps existing flat menu indices unchanged and gives every
/// nested item its own receipt. Separators and submenu headers are never actions.
pub fn menu_item(options: &[Value], index: usize) -> Option<&Value> {
    fn find<'a>(options: &'a [Value], index: usize, next: &mut usize) -> Option<&'a Value> {
        for option in options {
            let current = *next;
            *next += 1;
            if current == index {
                return Some(option);
            }
            if let Some(found) = find(children(option), index, next) {
                return Some(found);
            }
        }
        None
    }
    find(options, index, &mut 0)
}
fn children(option: &Value) -> &[Value] {
    option
        .get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}
fn append_menu(
    menu: &NSMenu,
    options: &[Value],
    target: &MenuTarget,
    mtm: MainThreadMarker,
    next: &mut usize,
) {
    use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn};
    for option in options {
        let index = *next;
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
        let descendants = children(option);
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(label),
                if descendants.is_empty() {
                    Some(sel!(choose:))
                } else {
                    None
                },
                &NSString::new(),
            )
        };
        item.setTag(index as isize);
        item.setEnabled(
            option.get("disabled") != Some(&Value::Bool(true))
                && option.get("enabled") != Some(&Value::Bool(false)),
        );
        item.setState(if option.get("checked") == Some(&Value::Bool(true)) {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        if let Some(tooltip) = option.get("tooltip").and_then(Value::as_str) {
            item.setToolTip(Some(&NSString::from_str(tooltip)));
        }
        unsafe {
            item.setTarget(Some(target));
        }
        if !descendants.is_empty() {
            let submenu = NSMenu::new(mtm);
            submenu.setAutoenablesItems(false);
            append_menu(&submenu, descendants, target, mtm, next);
            item.setSubmenu(Some(&submenu));
        }
        menu.addItem(&item);
    }
}
/// The same winning providers and saved recents used by the Effects panel.
pub fn effect_options(state: &pm_kernel::state::StateSnapshot) -> Vec<Value> {
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};
    let mut providers = BTreeMap::new();
    for entry in state.registries.get("effects").into_iter().flatten() {
        providers.insert(entry.id.clone(), &entry.definition);
    }
    let item = |id: &str, definition: &pm_model::Value| json!({"id":id,"label":definition.get("label").and_then(pm_model::Value::as_str).unwrap_or(id)});
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    let recent = state
        .settings
        .get("ext.builtin.effects")
        .and_then(|v| v.get("pm.fx.recent"))
        .and_then(pm_model::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(pm_model::Value::as_str)
        .filter(|id| seen.insert((*id).to_owned()))
        .filter_map(|id| providers.get(id).map(|definition| item(id, definition)))
        .take(8)
        .collect::<Vec<_>>();
    if !recent.is_empty() {
        result.push(json!({"label":"Recent","children":recent}));
    }
    let mut groups = BTreeMap::<String, Vec<Value>>::new();
    for (id, definition) in providers {
        let group = definition
            .get("group")
            .and_then(pm_model::Value::as_str)
            .unwrap_or("");
        groups
            .entry(group.into())
            .or_default()
            .push(item(&id, definition));
    }
    for (group, items) in groups {
        if group.is_empty() {
            result.extend(items);
        } else {
            result.push(json!({"label":group,"children":items}));
        }
    }
    result
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

#[cfg(test)]
mod menu_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn nested_choices_keep_flat_receipts_and_do_not_alias_the_next_action() {
        let options = vec![
            json!({"id":"a"}),
            Value::Null,
            json!({"label":"Group","children":[json!({"id":"b"}),json!({"label":"Nested","children":[json!({"id":"c"})]})]}),
            json!({"id":"d"}),
        ];
        for (index, id) in [(0, "a"), (3, "b"), (5, "c"), (6, "d")] {
            assert_eq!(menu_item(&options, index).unwrap()["id"], id);
        }
        assert!(menu_item(&options, 7).is_none());
    }
    #[test]
    fn effect_groups_use_the_winning_provider_and_only_retained_recent_ids() {
        use pm_kernel::{
            Writer,
            state::{Registration, StateOp, StateTransaction},
        };
        let mut state = pm_kernel::state::StateSnapshot::default();
        for (token, id, label, group) in [
            ("first", "a", "Old", "Blur"),
            ("second", "a", "New", "Light"),
            ("third", "b", "Other", "Blur"),
        ] {
            state = state
                .stage(StateTransaction {
                    expected_revision: None,
                    ops: vec![StateOp::Register {
                        registry: "effects".into(),
                        entry: Registration {
                            token: token.into(),
                            owner: Writer {
                                realm: 1,
                                generation: 1,
                            },
                            id: id.into(),
                            definition: pm_model::json::parse(
                                &json!({"label":label,"group":group}).to_string(),
                            )
                            .unwrap(),
                        },
                    }],
                })
                .unwrap()
                .0;
        }
        std::sync::Arc::make_mut(&mut state.settings).insert(
            "ext.builtin.effects".into(),
            pm_model::json::parse(r#"{"pm.fx.recent":["missing","a","a","b"]}"#).unwrap(),
        );
        let options = effect_options(&state);
        assert_eq!(options[0]["label"], "Recent");
        assert_eq!(options[0]["children"].as_array().unwrap().len(), 2);
        assert_eq!(options[0]["children"][0]["label"], "New");
        assert_eq!(options[1]["label"], "Blur");
        assert_eq!(options[1]["children"].as_array().unwrap().len(), 1);
        assert_eq!(options[2]["label"], "Light");
    }
}
