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
use std::{cell::RefCell, path::PathBuf};

#[link(name = "Quartz", kind = "framework")]
unsafe extern "C" {}

/// What Quick Look shows: the files, in the selection's order.
#[derive(Default)]
pub struct Items(RefCell<Vec<Retained<NSURL>>>);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMQuickLookSource"]
    #[ivars = Items]
    struct Source;
    impl Source {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn count(&self, _panel: *mut AnyObject) -> isize {
            self.ivars().0.borrow().len() as isize
        }
        #[unsafe(method(previewPanel:previewItemAtIndex:))]
        fn item(&self, _panel: *mut AnyObject, index: isize) -> *mut AnyObject {
            self.ivars()
                .0
                .borrow()
                .get(index.max(0) as usize)
                .map_or(std::ptr::null_mut(), |url| {
                    Retained::as_ptr(url) as *mut AnyObject
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

/// Space on the selection: Quick Look of `files` (closed again if it is
/// showing). Files that are gone are left out; none left, nothing opens.
pub fn toggle(files: Vec<PathBuf>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    install(mtm);
    if showing() {
        let _: () = unsafe { msg_send![panel(), orderOut: std::ptr::null::<AnyObject>()] };
        return;
    }
    let urls: Vec<Retained<NSURL>> = files
        .iter()
        .filter(|p| p.exists())
        .map(|p| NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy())))
        .collect();
    if urls.is_empty() {
        return;
    }
    *source(mtm).ivars().0.borrow_mut() = urls;
    let panel = panel();
    if panel.is_null() {
        return;
    }
    unsafe {
        let _: () = msg_send![panel, updateController];
        let _: () = msg_send![panel, reloadData];
        let _: () = msg_send![panel, setCurrentPreviewItemIndex: 0isize];
        let _: () = msg_send![panel, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
    }
}

/// The editor windows' Quick Look control (added once to GPUI's window
/// class): they accept it while there is something to show and hand the
/// panel this module's source.
fn install(mtm: MainThreadMarker) {
    thread_local!(static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    let _ = source(mtm);
    extern "C-unwind" fn accepts(_: *mut AnyObject, _: Sel, _panel: *mut AnyObject) -> Bool {
        let items = SOURCE.with(|s| s.borrow().as_ref().map_or(0, |s| s.ivars().0.borrow().len()));
        Bool::new(items > 0)
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
    for name in [c"GPUIWindow", c"GPUIPanel"] {
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
