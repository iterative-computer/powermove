//! Powermove P-A11Y fallback for AccessKit 0.26's closed platform-node tree.
//! Keep the upstream adapter and its real root/children. Wrap only the content
//! view and (when slots exist) its AccessKit root, using ivar-free subclasses.
//! Native descendants keep their own frame, focus, text and action implementations.
use crate::composition::{self, AccessibilityItem};
use crate::composition_accessibility_order::{Entry, merge_order};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, ClassBuilder, Sel},
    sel,
};
use objc2_app_kit::NSView;
use objc2_foundation::{NSArray, NSPoint, NSString};
use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::{CString, c_void},
    rc::{Rc, Weak},
};

// Restoring an ivar-free subclass is the inverse of its installation (the
// upstream SubclassingAdapter uses the same runtime operation). Use the raw
// API: AnyObject::set_class's contract only permits installing a subclass.
unsafe fn restore_class(object: &AnyObject, previous: &AnyClass) {
    unsafe {
        objc2::ffi::object_setClass((object as *const AnyObject).cast_mut(), previous);
    }
}

struct WrappedRoot {
    object: Retained<AnyObject>,
    previous: &'static AnyClass,
}
impl Drop for WrappedRoot {
    fn drop(&mut self) {
        ROOTS.with(|roots| {
            roots
                .borrow_mut()
                .remove(&(Retained::as_ptr(&self.object) as usize));
        });
        // No ivars added; restore before releasing the AccessKit node/context.
        unsafe {
            restore_class(&self.object, self.previous);
        }
    }
}
struct State {
    gpui: usize,
    content: Retained<NSView>,
    previous: &'static AnyClass,
    root: Option<WrappedRoot>,
}
thread_local! {
    static VIEWS: RefCell<HashMap<usize, Weak<RefCell<State>>>> = RefCell::new(HashMap::new());
    static ROOTS: RefCell<HashMap<usize, Weak<RefCell<State>>>> = RefCell::new(HashMap::new());
}
fn state(object: &AnyObject, root: bool) -> Rc<RefCell<State>> {
    let key = object as *const AnyObject as usize;
    let find = |map: &RefCell<HashMap<usize, Weak<RefCell<State>>>>| {
        map.borrow().get(&key).and_then(Weak::upgrade)
    };
    if root {
        ROOTS.with(find)
    } else {
        VIEWS.with(find)
    }
    .expect("live composition AX adapter")
}
fn subclass(previous: &'static AnyClass, root: bool) -> &'static AnyClass {
    let name = CString::new(format!(
        "PMCompositionAX{}Of{}",
        if root { "Root" } else { "View" },
        previous.name().to_string_lossy()
    ))
    .unwrap();
    if let Some(class) = AnyClass::get(&name) {
        return class;
    }
    let mut builder = ClassBuilder::new(&name, previous).unwrap();
    unsafe {
        builder.add_method(
            sel!(accessibilityChildren),
            children as unsafe extern "C" fn(_, _) -> _,
        );
        builder.add_method(
            sel!(accessibilityChildrenInNavigationOrder),
            children as unsafe extern "C" fn(_, _) -> _,
        );
        if !root {
            builder.add_method(
                sel!(accessibilityHitTest:),
                hit_test as unsafe extern "C" fn(_, _, _) -> _,
            );
            builder.add_method(
                sel!(accessibilityFocusedUIElement),
                focus as unsafe extern "C" fn(_, _) -> _,
            );
        }
    }
    builder.register()
}
fn native_views(gpui: usize) -> Vec<Retained<NSView>> {
    composition::accessibility(gpui)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            if let AccessibilityItem::Native(view) = entry {
                Some(view)
            } else {
                None
            }
        })
        .collect()
}
fn identifier(object: &AnyObject) -> Option<String> {
    unsafe {
        let responds: bool = msg_send![object, respondsToSelector: sel!(accessibilityIdentifier)];
        if !responds {
            return None;
        }
        let value: Option<Retained<NSString>> = msg_send![object, accessibilityIdentifier];
        value.map(|s| s.to_string())
    }
}
unsafe extern "C" fn children(this: &AnyObject, _: Sel) -> *mut NSArray<AnyObject> {
    let is_root = ROOTS.with(|roots| roots.borrow().contains_key(&(this as *const _ as usize)));
    let state = state(this, is_root);
    let (previous, gpui) = {
        let state = state.borrow();
        (
            if is_root {
                state.root.as_ref().unwrap().previous
            } else {
                state.previous
            },
            state.gpui,
        )
    };
    let original: Option<Retained<NSArray<AnyObject>>> =
        unsafe { msg_send![super(this, previous), accessibilityChildren] };
    let Some(order) = composition::accessibility(gpui) else {
        return original.map_or(std::ptr::null_mut(), Retained::autorelease_return);
    };
    if order.is_empty() {
        return original.map_or(std::ptr::null_mut(), Retained::autorelease_return);
    }
    if !is_root {
        // AccessKit exposes a single Role::Window root. Preserve it and all
        // GPUI ancestry: flattening that node would leave incorrect AXParents.
        if let Some(array) = &original {
            if array.len() == 1 {
                let root = array.objectAtIndex(0);
                let unchanged = state
                    .borrow()
                    .root
                    .as_ref()
                    .is_some_and(|r| std::ptr::eq(&*r.object, &*root));
                if !unchanged {
                    composition::clear_parents(&order);
                    let old = state.borrow_mut().root.take();
                    drop(old);
                    let previous: &'static AnyClass =
                        unsafe { &*(root.class() as *const AnyClass) };
                    let class = subclass(previous, true);
                    ROOTS.with(|roots| {
                        roots
                            .borrow_mut()
                            .insert(Retained::as_ptr(&root) as usize, Rc::downgrade(&state));
                    });
                    unsafe {
                        AnyObject::set_class(&root, class);
                    }
                    state.borrow_mut().root = Some(WrappedRoot {
                        object: root,
                        previous,
                    });
                }
            }
        }
        return original.map_or(std::ptr::null_mut(), Retained::autorelease_return);
    }
    let gpui_children: Vec<_> = original.into_iter().flat_map(|a| a.to_vec()).collect();
    let ids: Vec<_> = gpui_children.iter().map(|c| identifier(c)).collect();
    let mut result = Vec::new();
    let mut emitted = std::collections::HashSet::new();
    let entries: Vec<_> = order
        .iter()
        .map(|item| match item {
            AccessibilityItem::Gpui(id) => Entry::Gpui(id.clone()),
            AccessibilityItem::Native(_) => Entry::Native,
            AccessibilityItem::RemainingGpui => Entry::RemainingGpui,
        })
        .collect();
    for (native, index) in merge_order(&ids, &entries) {
        let candidates = if native {
            let AccessibilityItem::Native(view) = &order[index] else {
                unreachable!()
            };
            if view.isHiddenOrHasHiddenAncestor() || view.window().is_none() {
                continue;
            }
            // NSControl may expose its cell, not the NSView itself. Returning
            // an ignored view creates an AXUnknown wrapper/duplicate identity.
            let children =
                unsafe { objc2_app_kit::NSAccessibilityUnignoredChildrenForOnlyChild(view) };
            children.to_vec()
        } else {
            vec![gpui_children[index].clone()]
        };
        for child in candidates {
            if native {
                unsafe {
                    let _: () = msg_send![&*child, setAccessibilityParent: this];
                }
            }
            if emitted.insert(Retained::as_ptr(&child) as usize) {
                result.push(child);
            }
        }
    }
    Retained::autorelease_return(NSArray::from_retained_slice(&result))
}
unsafe extern "C" fn hit_test(this: &AnyObject, _: Sel, point: NSPoint) -> *mut AnyObject {
    let state = state(this, false);
    let (gpui, content, previous) = {
        let s = state.borrow();
        (s.gpui, s.content.clone(), s.previous)
    };
    if let Some(window) = content.window() {
        let window_point = window.convertPointFromScreen(point);
        let parent = unsafe { content.superview() };
        let local = content.convertPoint_fromView(window_point, None);
        let parent_point = parent.as_ref().map_or(window_point, |parent| {
            parent.convertPoint_fromView(local, Some(&content))
        });
        // Ordinary AppKit hit testing already implements the composition's
        // z-order, transparent GPUI holes and SlotView clip masks.
        let hit: Option<Retained<NSView>> = unsafe { msg_send![&*content, hitTest: parent_point] };
        if let Some(hit) = hit {
            for view in native_views(gpui) {
                if std::ptr::eq(&*hit, &*view) || hit.isDescendantOf(&view) {
                    return unsafe { msg_send![&*view, accessibilityHitTest: point] };
                }
            }
        }
    }
    unsafe { msg_send![super(this, previous), accessibilityHitTest: point] }
}
unsafe extern "C" fn focus(this: &AnyObject, _: Sel) -> *mut AnyObject {
    let state = state(this, false);
    let (gpui, content, previous) = {
        let s = state.borrow();
        (s.gpui, s.content.clone(), s.previous)
    };
    if let Some(window) = content.window() {
        if let Some(responder) = window.firstResponder() {
            for view in native_views(gpui) {
                let descendant: bool = unsafe {
                    let is_view: bool =
                        msg_send![&*responder, isKindOfClass: objc2::class!(NSView)];
                    is_view && msg_send![&*responder, isDescendantOf: &*view]
                };
                let editor: *mut AnyObject = unsafe {
                    let has_editor: bool =
                        msg_send![&*view, respondsToSelector: sel!(currentEditor)];
                    if has_editor {
                        msg_send![&*view, currentEditor]
                    } else {
                        std::ptr::null_mut()
                    }
                };
                if descendant || std::ptr::eq(editor, Retained::as_ptr(&responder).cast()) {
                    // WebKit's responder knows the focused remote DOM node;
                    // the containing slot group would just return itself.
                    // For an NSControl's shared field editor, ask the control
                    // so focus reports its text field rather than the editor.
                    let focused: *mut AnyObject =
                        if std::ptr::eq(editor, Retained::as_ptr(&responder).cast()) {
                            unsafe { msg_send![&*view, accessibilityFocusedUIElement] }
                        } else {
                            unsafe { msg_send![&*responder, accessibilityFocusedUIElement] }
                        };
                    return if focused.is_null() {
                        Retained::as_ptr(&view) as *mut _
                    } else {
                        focused
                    };
                }
            }
        }
    }
    unsafe { msg_send![super(this, previous), accessibilityFocusedUIElement] }
}

/// Drop restores our subclasses before SubclassingAdapter restores its own.
/// Neither callbacks nor native AX queries acquire MacWindowState's mutex.
pub(crate) struct Adapter {
    state: Rc<RefCell<State>>,
    upstream: accesskit_macos::SubclassingAdapter,
}
impl Adapter {
    pub unsafe fn for_window(
        window: *mut c_void,
        gpui: usize,
        activation: impl 'static + accesskit::ActivationHandler,
        actions: impl 'static + accesskit::ActionHandler,
    ) -> Self {
        let upstream =
            unsafe { accesskit_macos::SubclassingAdapter::for_window(window, activation, actions) };
        let content: Retained<NSView> = unsafe { msg_send![window as *mut AnyObject, contentView] };
        let previous: &'static AnyClass = unsafe { &*(content.class() as *const AnyClass) };
        let class = subclass(previous, false);
        let state = Rc::new(RefCell::new(State {
            gpui,
            content,
            previous,
            root: None,
        }));
        let content = state.borrow().content.clone();
        VIEWS.with(|views| {
            views
                .borrow_mut()
                .insert(Retained::as_ptr(&content) as usize, Rc::downgrade(&state));
        });
        unsafe {
            AnyObject::set_class(&content, class);
        }
        Self { state, upstream }
    }
    pub fn update_if_active(
        &mut self,
        update: impl FnOnce() -> accesskit::TreeUpdate,
    ) -> Option<accesskit_macos::QueuedEvents> {
        self.upstream.update_if_active(update)
    }
    pub fn update_view_focus_state(
        &mut self,
        focused: bool,
    ) -> Option<accesskit_macos::QueuedEvents> {
        self.upstream.update_view_focus_state(focused)
    }
}
impl Drop for Adapter {
    fn drop(&mut self) {
        let gpui = self.state.borrow().gpui;
        composition::clear_parents(&composition::accessibility(gpui).unwrap_or_default());
        let (content, previous, root) = {
            let mut state = self.state.borrow_mut();
            (state.content.clone(), state.previous, state.root.take())
        };
        unsafe {
            restore_class(&content, previous);
        }
        VIEWS.with(|views| {
            views
                .borrow_mut()
                .remove(&(Retained::as_ptr(&content) as usize));
        });
        drop(root);
    }
}

/// Deliver structural changes after GPUI releases its draw lock. Only the
/// window key crosses the queue; main-thread-only AppKit objects stay here.
pub(crate) fn changed(gpui: usize) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        let states: Vec<_> =
            VIEWS.with(|views| views.borrow().values().filter_map(Weak::upgrade).collect());
        for state in states {
            let state = state.borrow();
            if state.gpui != gpui {
                continue;
            }
            let target = state.root.as_ref().map_or_else(
                || {
                    Retained::into_super(Retained::into_super(Retained::into_super(
                        state.content.clone(),
                    )))
                },
                |root| root.object.clone(),
            );
            drop(state);
            unsafe {
                objc2_app_kit::NSAccessibilityPostNotification(
                    &target,
                    objc2_app_kit::NSAccessibilityLayoutChangedNotification,
                );
            }
        }
    });
}

/// The drawing view must keep ordinary AppKit behavior when accessibility was
/// explicitly disabled (there is then no content-view adapter to forward to).
pub(crate) fn has_adapter(content: usize) -> bool {
    VIEWS.with(|views| {
        views
            .borrow()
            .get(&content)
            .is_some_and(|state| state.strong_count() > 0)
    })
}
