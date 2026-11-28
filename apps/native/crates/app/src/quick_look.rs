//! Space on the Project panel's selection: Apple's Quick Look of the original
//! files, as Finder shows it. The arrow keys move through the selection while
//! it is open (the panel's own navigation over its items); Space or Escape
//! closes it. The editor windows take Quick Look's control through the
//! responder chain (`acceptsPreviewPanelControl:`), as Finder's do.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool, NSObject, Sel},
    sel,
};
use objc2_foundation::{NSString, NSURL};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};

#[link(name = "Quartz", kind = "framework")]
unsafe extern "C" {}

/// What Quick Look shows: the files, in the selection's order.
#[derive(Default)]
pub struct Items {
    opening: Cell<bool>,
    files: RefCell<Vec<(PathBuf, String)>>,
    previews: RefCell<Vec<Retained<Item>>>,
}

/// One preview: the original file under the name the Project shows (an
/// original in the library's store is named by its bytes, not by the file).
pub struct ItemData {
    url: Retained<NSURL>,
    title: Retained<NSString>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMQuickLookItem"]
    #[ivars = ItemData]
    struct Item;
    impl Item {
        #[unsafe(method(previewItemURL))]
        fn url(&self) -> *mut NSURL {
            Retained::as_ptr(&self.ivars().url) as *mut NSURL
        }
        #[unsafe(method(previewItemTitle))]
        fn title(&self) -> *mut NSString {
            Retained::as_ptr(&self.ivars().title) as *mut NSString
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMQuickLookSource"]
    #[ivars = Items]
    struct Source;
    impl Source {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn count(&self, _panel: *mut AnyObject) -> isize {
            self.ivars().previews.borrow().len() as isize
        }
        #[unsafe(method(previewPanel:previewItemAtIndex:))]
        fn item(&self, _panel: *mut AnyObject, index: isize) -> *mut AnyObject {
            self.ivars()
                .previews
                .borrow()
                .get(index.max(0) as usize)
                .map_or(std::ptr::null_mut(), |item| {
                    Retained::as_ptr(item) as *mut AnyObject
                })
        }
        /// Space closes the preview, as in Finder (Escape closes it already).
        #[unsafe(method(previewPanel:handleEvent:))]
        fn handle_event(&self, panel: *mut AnyObject, event: *mut AnyObject) -> Bool {
            if event.is_null() || panel.is_null() {
                return Bool::NO;
            }
            let kind: usize = unsafe { msg_send![event, type] };
            // NSEventTypeKeyDown
            if kind != 10 {
                return Bool::NO;
            }
            let characters: *mut NSString = unsafe { msg_send![event, charactersIgnoringModifiers] };
            if !characters.is_null() && unsafe { &*characters }.to_string() == " " {
                let _: () = unsafe { msg_send![panel, orderOut: std::ptr::null::<AnyObject>()] };
                return Bool::YES;
            }
            Bool::NO
        }
    }
);

thread_local! {
    static SOURCE: RefCell<Option<Retained<Source>>> = const { RefCell::new(None) };
}

fn source(mtm: MainThreadMarker) -> Retained<Source> {
    SOURCE.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(|| unsafe {
                msg_send![super(Source::alloc(mtm).set_ivars(Items::default())), init]
            })
            .clone()
    })
}

fn panel() -> *mut AnyObject {
    let Some(class) = AnyClass::get(c"QLPreviewPanel") else {
        return std::ptr::null_mut();
    };
    unsafe { msg_send![class, sharedPreviewPanel] }
}

/// Whether Quick Look is showing.
pub fn showing() -> bool {
    let has: Bool = match AnyClass::get(c"QLPreviewPanel") {
        Some(class) => unsafe { msg_send![class, sharedPreviewPanelExists] },
        None => return false,
    };
    has.as_bool() && {
        let visible: Bool = unsafe { msg_send![panel(), isVisible] };
        visible.as_bool()
    }
}

/// Space on the selection: Quick Look of `files` (path and the name the
/// Project shows; closed again if it is showing). Files that are gone are
/// left out; none left, nothing opens.
pub fn toggle(files: Vec<(PathBuf, String)>) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    if showing() {
        hide();
    } else {
        show(files);
    }
}

fn hide() {
    let _: () = unsafe { msg_send![panel(), orderOut: std::ptr::null::<AnyObject>()] };
}

/// Clicking an attachment toggles its own preview or replaces the open one.
pub fn show(files: Vec<(PathBuf, String)>) {
    show_at(files, 0);
}

/// Show Quick Look over `files`, starting at `initial_index`. Clicking the
/// currently displayed item toggles Quick Look closed; clicking a sibling
/// jumps directly to it so arrow keys browse the rest of the set.
pub fn show_at(files: Vec<(PathBuf, String)>, initial_index: usize) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let mut kept = Vec::with_capacity(files.len());
    let mut index = 0usize;
    for (i, (path, name)) in files.into_iter().enumerate() {
        if path.is_file() {
            if i == initial_index {
                index = kept.len();
            }
            kept.push((path, name));
        }
    }
    if kept.is_empty() {
        return;
    }
    let index = index.min(kept.len() - 1);
    let source = source(mtm);
    let panel = panel();
    if panel.is_null() {
        return;
    }
    if showing() && has_source(panel, &source) && *source.ivars().files.borrow() == kept {
        let current: isize = unsafe { msg_send![panel, currentPreviewItemIndex] };
        if current == index as isize {
            hide();
        } else {
            unsafe {
                let _: () = msg_send![panel, setCurrentPreviewItemIndex: index as isize];
                let _: () = msg_send![panel, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
            }
        }
        return;
    }
    install();
    *source.ivars().previews.borrow_mut() = kept
        .iter()
        .map(|(path, name)| {
            let data = ItemData {
                url: NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())),
                title: NSString::from_str(name),
            };
            unsafe { msg_send![super(Item::alloc(mtm).set_ivars(data)), init] }
        })
        .collect();
    *source.ivars().files.borrow_mut() = kept;
    source.ivars().opening.set(true);
    unsafe {
        // Becoming key can change the responder chain, including when an
        // AppKit field held focus in a non-main popover. Bind to the controller
        // the panel resolves after that transition.
        let _: () = msg_send![panel, updateController];
        let _: () = msg_send![panel, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
        let _: () = msg_send![panel, updateController];
        if has_source(panel, &source) {
            let _: () = msg_send![panel, reloadData];
            let _: () = msg_send![panel, setCurrentPreviewItemIndex: index as isize];
        }
    }
    source.ivars().opening.set(false);
}

fn has_source(panel: *mut AnyObject, source: &Retained<Source>) -> bool {
    let data_source: *mut AnyObject = unsafe { msg_send![panel, dataSource] };
    std::ptr::eq(data_source, Retained::as_ptr(source).cast::<AnyObject>())
}

/// The docked view and popover both use GPUI windows. The application
/// provides a final responder if a native control or the preview takes key.
fn install() {
    extern "C-unwind" fn accepts(_: *mut AnyObject, _: Sel, panel: *mut AnyObject) -> Bool {
        let visible: Bool = unsafe { msg_send![panel, isVisible] };
        SOURCE.with(|s| {
            Bool::new(s.borrow().as_ref().is_some_and(|source| {
                let items = source.ivars();
                (items.opening.get() || visible.as_bool()) && !items.previews.borrow().is_empty()
            }))
        })
    }
    extern "C-unwind" fn begin(_: *mut AnyObject, _: Sel, panel: *mut AnyObject) {
        SOURCE.with(|s| {
            if let Some(source) = s.borrow().as_ref() {
                let object = Retained::as_ptr(source) as *mut AnyObject;
                unsafe {
                    let _: () = msg_send![panel, setDataSource: object];
                    let _: () = msg_send![panel, setDelegate: object];
                }
            }
        });
    }
    extern "C-unwind" fn end(_: *mut AnyObject, _: Sel, panel: *mut AnyObject) {
        unsafe {
            let _: () = msg_send![panel, setDataSource: std::ptr::null::<AnyObject>()];
            let _: () = msg_send![panel, setDelegate: std::ptr::null::<AnyObject>()];
        }
    }
    // Add only to our own classes. A panel class may be registered after
    // the first preview; class_addMethod leaves installed methods in place.
    for name in [
        c"GPUIWindow",
        c"GPUIPanel",
        c"GPUIApplication",
        c"GPUIApplicationDelegate",
    ] {
        let Some(class) = AnyClass::get(name) else {
            continue;
        };
        let class = class as *const AnyClass as *mut AnyClass;
        unsafe {
            objc2::ffi::class_addMethod(
                class,
                sel!(acceptsPreviewPanelControl:),
                std::mem::transmute::<
                    extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool,
                    objc2::runtime::Imp,
                >(accepts),
                c"B@:@".as_ptr(),
            );
            objc2::ffi::class_addMethod(
                class,
                sel!(beginPreviewPanelControl:),
                std::mem::transmute::<
                    extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                    objc2::runtime::Imp,
                >(begin),
                c"v@:@".as_ptr(),
            );
            objc2::ffi::class_addMethod(
                class,
                sel!(endPreviewPanelControl:),
                std::mem::transmute::<
                    extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                    objc2::runtime::Imp,
                >(end),
                c"v@:@".as_ptr(),
            );
        }
    }
}
