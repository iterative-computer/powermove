//! Exercise the exact vendored bridge without showing a window. In particular,
//! validate NSTextView, publication/removal, and restoring Objective-C classes.
#[path = "../../../vendor/gpui-ce/crates/gpui_macos/src/composition.rs"]
mod composition;
#[path = "../../../vendor/gpui-ce/crates/gpui_macos/src/composition_accessibility.rs"]
mod composition_accessibility;
#[path = "../../../vendor/gpui-ce/crates/gpui_macos/src/composition_accessibility_order.rs"]
mod composition_accessibility_order;
use composition::{AccessibilityItem as Item, publish_accessibility};
use objc2::{MainThreadMarker, MainThreadOnly, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{
    NSAccessibility, NSApplication, NSBackingStoreType, NSButton, NSTextField, NSTextView, NSView,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};
use std::rc::Rc;
fn rect(x: f64, y: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(100., 30.))
}
fn tree() -> accesskit::TreeUpdate {
    let mut root = accesskit::Node::new(accesskit::Role::Window);
    root.set_children(vec![accesskit::NodeId(2), accesskit::NodeId(3)]);
    let mut before = accesskit::Node::new(accesskit::Role::Group);
    before.set_author_id("before");
    let mut after = accesskit::Node::new(accesskit::Role::Group);
    after.set_author_id("after");
    accesskit::TreeUpdate {
        nodes: vec![
            (accesskit::NodeId(1), root),
            (accesskit::NodeId(2), before),
            (accesskit::NodeId(3), after),
        ],
        tree: Some(accesskit::Tree::new(accesskit::NodeId(1))),
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(2),
    }
}
struct Activation;
impl accesskit::ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<accesskit::TreeUpdate> {
        Some(tree())
    }
}
struct Actions;
impl accesskit::ActionHandler for Actions {
    fn do_action(&mut self, _: accesskit::ActionRequest) {}
}
unsafe fn children(view: &AnyObject) -> Retained<NSArray<AnyObject>> {
    unsafe { msg_send![view, accessibilityChildren] }
}
unsafe fn ids(view: &AnyObject) -> Vec<String> {
    unsafe { children(view) }
        .to_vec()
        .iter()
        .map(|child| {
            let id: Option<Retained<NSString>> =
                unsafe { msg_send![&**child, accessibilityIdentifier] };
            id.map_or(String::new(), |s| s.to_string())
        })
        .collect()
}
fn main() {
    let mtm = MainThreadMarker::new().unwrap();
    let _app = NSApplication::sharedApplication(mtm);
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(640., 480.)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe {
        window.setReleasedWhenClosed(false);
    }
    let content = window.contentView().unwrap();
    let original_class = content.class();
    let gpui = NSView::initWithFrame(NSView::alloc(mtm), content.bounds());
    content.addSubview(&gpui);
    let field = NSTextField::initWithFrame(NSTextField::alloc(mtm), rect(10., 10.));
    field.setAccessibilityIdentifier(Some(&NSString::from_str("field")));
    content.addSubview(&field);
    let text = NSTextView::initWithFrame(NSTextView::alloc(mtm), rect(10., 60.));
    text.setAccessibilityIdentifier(Some(&NSString::from_str("multiline")));
    content.addSubview(&text);
    let button = NSButton::initWithFrame(NSButton::alloc(mtm), rect(10., 110.));
    button.setAccessibilityIdentifier(Some(&NSString::from_str("button")));
    content.addSubview(&button);
    let key = Retained::as_ptr(&gpui) as usize;
    let mut adapter = unsafe {
        composition_accessibility::Adapter::for_window(
            Retained::as_ptr(&window) as *mut _,
            key,
            Activation,
            Actions,
        )
    };
    let registration = composition::register(&gpui, Rc::new(|| {}), Rc::new(|_| true));
    publish_accessibility(
        &gpui,
        vec![
            Item::Gpui("before".into()),
            Item::Native(Retained::into_super(Retained::into_super(field.clone()))),
            Item::Native(Retained::into_super(Retained::into_super(text.clone()))),
            Item::Native(Retained::into_super(Retained::into_super(button.clone()))),
            Item::RemainingGpui,
        ],
    );
    let root = unsafe { children(&content) }.objectAtIndex(0);
    let order = unsafe { ids(&root) };
    assert_eq!(order.first().map(String::as_str), Some("before"));
    assert_eq!(order.last().map(String::as_str), Some("after"));
    let roles: Vec<String> = unsafe { children(&root) }
        .to_vec()
        .iter()
        .map(|child| {
            let role: Retained<NSString> = unsafe { msg_send![&**child, accessibilityRole] };
            role.to_string()
        })
        .collect();
    assert_eq!(
        roles,
        [
            "AXGroup",
            "AXTextField",
            "AXTextArea",
            "AXButton",
            "AXGroup"
        ]
    );
    let direct = unsafe { children(&root) };
    let navigation: Retained<NSArray<AnyObject>> =
        unsafe { msg_send![&*root, accessibilityChildrenInNavigationOrder] };
    assert_eq!(direct.len(), navigation.len());
    assert!(
        direct
            .to_vec()
            .iter()
            .zip(navigation.to_vec())
            .all(|(a, b)| std::ptr::eq(&**a, &*b))
    );
    for child in direct.to_vec() {
        let parent: Retained<AnyObject> = unsafe { msg_send![&*child, accessibilityParent] };
        assert!(std::ptr::eq(&*parent, &*root));
    }
    println!(
        "PASS bridge native NSTextField/NSTextView/NSButton order, parents and navigation selector"
    );
    // Reorder, removing the field and button without destroying their views.
    publish_accessibility(
        &gpui,
        vec![
            Item::Gpui("after".into()),
            Item::Native(Retained::into_super(Retained::into_super(text.clone()))),
            Item::RemainingGpui,
        ],
    );
    assert_eq!(unsafe { ids(&root) }, ["after", "multiline", "before"]);
    let exposed = unsafe { objc2_app_kit::NSAccessibilityUnignoredChildrenForOnlyChild(&field) };
    for child in exposed.to_vec() {
        let parent: Option<Retained<AnyObject>> =
            unsafe { msg_send![&*child, accessibilityParent] };
        assert!(!parent.is_some_and(|p| std::ptr::eq(&*p, &*root)));
    }
    println!("PASS bridge reorder/removal clears old native parent");
    if let Some(events) = adapter.update_if_active(tree) {
        events.raise();
    }
    assert_eq!(unsafe { ids(&root) }, ["after", "multiline", "before"]);
    drop(registration);
    assert_eq!(unsafe { ids(&root) }, ["before", "after"]);
    println!("PASS bridge AccessKit update and unregister restore GPUI-only children");
    drop(adapter);
    assert!(std::ptr::eq(content.class(), original_class));
    assert_eq!(root.class().name().to_string_lossy(), "AccessKitNode");
    println!("PASS bridge teardown restores content/root classes while AX objects are retained");
    println!("S2b BRIDGE RESULT PASS (window never ordered on screen)");
}
